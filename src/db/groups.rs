//! Groups and the rules between them. A group has an **owner** (none for the system group
//! `reef`), and an **access rule** says "traffic from this source group may reach this target
//! group". Each rule is backed by a NetBird policy, whose id we keep so we can delete it.

use sqlx::SqlitePool;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct Group {
    pub id: i64,
    pub name: String,
    /// `None` only for system groups (`reef`).
    pub owner_id: Option<i64>,
    pub netbird_id: String,
}

/// "Traffic from `source_group_id` to `target_group_id` is allowed."
#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct GroupAccess {
    pub id: i64,
    pub target_group_id: i64,
    pub source_group_id: i64,
    pub netbird_policy_id: String,
}

const GROUP_COLS: &str = "id, name, owner_id, netbird_id";
const ACCESS_COLS: &str = "id, target_group_id, source_group_id, netbird_policy_id";

// ---- groups ----

/// The group with this exact name, or `None`.
pub async fn by_name(db: &SqlitePool, name: &str) -> sqlx::Result<Option<Group>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {GROUP_COLS} from groups where name = ?"
    )))
    .bind(name)
    .fetch_optional(db)
    .await
}

/// One group by id, or `None`.
pub async fn by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<Group>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {GROUP_COLS} from groups where id = ?"
    )))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Every group, including `reef` and other people's (valid sources for an allow rule).
pub async fn all(db: &SqlitePool) -> sqlx::Result<Vec<Group>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {GROUP_COLS} from groups order by name"
    )))
    .fetch_all(db)
    .await
}

/// The groups one user owns.
pub async fn owned_by(db: &SqlitePool, owner: i64) -> sqlx::Result<Vec<Group>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {GROUP_COLS} from groups where owner_id = ? order by name"
    )))
    .bind(owner)
    .fetch_all(db)
    .await
}

/// A unique violation (see `db::is_unique_violation`) means the name is taken.
pub async fn insert(
    db: &SqlitePool,
    name: &str,
    owner: Option<i64>,
    netbird_id: &str,
) -> sqlx::Result<Group> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into groups (name, owner_id, netbird_id) values (?, ?, ?) returning {GROUP_COLS}"
    )))
    .bind(name)
    .bind(owner)
    .bind(netbird_id)
    .fetch_one(db)
    .await
}

/// Also removes access rules where the group is source or target (ON DELETE CASCADE).
pub async fn delete(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("delete from groups where id = ?")
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

// ---- access rules ----

/// Rules allowing traffic *into* `target`.
pub async fn access_into(db: &SqlitePool, target: i64) -> sqlx::Result<Vec<GroupAccess>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {ACCESS_COLS} from group_access where target_group_id = ? order by id"
    )))
    .bind(target)
    .fetch_all(db)
    .await
}

/// One access rule by id, or `None`.
pub async fn access_by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<GroupAccess>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {ACCESS_COLS} from group_access where id = ?"
    )))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// Record that `source` may now reach `target`, backed by NetBird policy `netbird_policy_id`.
/// A given pair can only have one rule (unique violation otherwise).
pub async fn insert_access(
    db: &SqlitePool,
    target: i64,
    source: i64,
    netbird_policy_id: &str,
) -> sqlx::Result<GroupAccess> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into group_access (target_group_id, source_group_id, netbird_policy_id) values (?, ?, ?) returning {ACCESS_COLS}"
    )))
    .bind(target)
    .bind(source)
    .bind(netbird_policy_id)
    .fetch_one(db)
    .await
}

/// Forget a rule (after deleting its NetBird policy).
pub async fn delete_access(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("delete from group_access where id = ?")
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{is_unique_violation, users::upsert_by_sub};

    async fn setup() -> (SqlitePool, i64, i64) {
        let db = crate::db::connect("sqlite::memory:").await.unwrap();
        let a = upsert_by_sub(&db, "a", "a@x", "a").await.unwrap().id;
        let b = upsert_by_sub(&db, "b", "b@x", "b").await.unwrap().id;
        (db, a, b)
    }

    #[tokio::test]
    async fn insert_returns_row_and_lookups_find_it() {
        let (db, a, _) = setup().await;
        let g = insert(&db, "team", Some(a), "nb-1").await.unwrap();
        assert_eq!(
            (g.name.as_str(), g.owner_id, g.netbird_id.as_str()),
            ("team", Some(a), "nb-1")
        );
        assert_eq!(by_name(&db, "team").await.unwrap().unwrap(), g);
        assert_eq!(by_id(&db, g.id).await.unwrap().unwrap(), g);
        assert!(
            by_name(&db, "Team").await.unwrap().is_none(),
            "names are case-sensitive here; normalisation is policy's job"
        );
        assert!(by_id(&db, 999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn duplicate_name_is_a_unique_violation_regardless_of_owner() {
        let (db, a, b) = setup().await;
        insert(&db, "team", Some(a), "nb-1").await.unwrap();
        let e = insert(&db, "team", Some(b), "nb-2").await.unwrap_err();
        assert!(is_unique_violation(&e), "{e}");
    }

    #[tokio::test]
    async fn system_group_has_no_owner_and_is_not_owned_by_anyone() {
        let (db, a, _) = setup().await;
        let reef = insert(&db, "reef", None, "nb-reef").await.unwrap();
        assert_eq!(reef.owner_id, None);
        assert!(owned_by(&db, a).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn all_and_owned_by() {
        let (db, a, b) = setup().await;
        insert(&db, "zz", Some(a), "1").await.unwrap();
        insert(&db, "aa", Some(b), "2").await.unwrap();
        insert(&db, "mm", Some(a), "3").await.unwrap();
        insert(&db, "reef", None, "4").await.unwrap();

        let names = |v: Vec<Group>| v.into_iter().map(|g| g.name).collect::<Vec<_>>();
        assert_eq!(names(all(&db).await.unwrap()), ["aa", "mm", "reef", "zz"]);
        assert_eq!(names(owned_by(&db, a).await.unwrap()), ["mm", "zz"]);
        assert_eq!(names(owned_by(&db, b).await.unwrap()), ["aa"]);
    }

    #[tokio::test]
    async fn access_rules_are_directional_and_listed_by_target() {
        let (db, a, b) = setup().await;
        let mine = insert(&db, "mine", Some(a), "n1").await.unwrap();
        let theirs = insert(&db, "theirs", Some(b), "n2").await.unwrap();

        let rule = insert_access(&db, mine.id, theirs.id, "pol-1")
            .await
            .unwrap();
        assert_eq!(
            (rule.target_group_id, rule.source_group_id),
            (mine.id, theirs.id)
        );

        assert_eq!(access_into(&db, mine.id).await.unwrap(), vec![rule.clone()]);
        assert!(
            access_into(&db, theirs.id).await.unwrap().is_empty(),
            "reverse direction was not granted"
        );
        assert_eq!(access_by_id(&db, rule.id).await.unwrap().unwrap(), rule);
    }

    #[tokio::test]
    async fn duplicate_access_pair_is_a_unique_violation() {
        let (db, a, b) = setup().await;
        let g1 = insert(&db, "g1", Some(a), "n1").await.unwrap();
        let g2 = insert(&db, "g2", Some(b), "n2").await.unwrap();
        insert_access(&db, g1.id, g2.id, "p1").await.unwrap();
        let e = insert_access(&db, g1.id, g2.id, "p2").await.unwrap_err();
        assert!(is_unique_violation(&e));
    }

    #[tokio::test]
    async fn delete_access_removes_only_that_rule() {
        let (db, a, b) = setup().await;
        let g1 = insert(&db, "g1", Some(a), "n1").await.unwrap();
        let g2 = insert(&db, "g2", Some(b), "n2").await.unwrap();
        let g3 = insert(&db, "g3", Some(b), "n3").await.unwrap();
        let r1 = insert_access(&db, g1.id, g2.id, "p1").await.unwrap();
        let r2 = insert_access(&db, g1.id, g3.id, "p2").await.unwrap();

        delete_access(&db, r1.id).await.unwrap();
        assert_eq!(access_into(&db, g1.id).await.unwrap(), vec![r2]);
        delete_access(&db, 999).await.unwrap();
    }

    #[tokio::test]
    async fn deleting_a_group_cascades_to_rules_in_both_roles() {
        let (db, a, b) = setup().await;
        let g1 = insert(&db, "g1", Some(a), "n1").await.unwrap();
        let g2 = insert(&db, "g2", Some(b), "n2").await.unwrap();
        insert_access(&db, g1.id, g2.id, "p1").await.unwrap();
        insert_access(&db, g2.id, g1.id, "p2").await.unwrap();

        delete(&db, g1.id).await.unwrap();

        assert!(by_id(&db, g1.id).await.unwrap().is_none());
        assert!(by_id(&db, g2.id).await.unwrap().is_some());
        assert!(access_into(&db, g2.id).await.unwrap().is_empty());
    }
}
