//! DNS ownership. A **claim** is a name a user owns (a TLD like `corp`, or a `.reef` domain like
//! `foo.reef`) together with everything beneath it; **records** live under a claim and are
//! mirrored to NetBird, whose ids we keep so we can later change or delete them there.

use sqlx::SqlitePool;

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct DnsClaim {
    pub id: i64,
    pub owner_id: i64,
    pub kind: String,
    pub name: String,
    /// Zone we created for a `tld` claim; `None` for `reef` claims (they live in the shared zone).
    pub netbird_zone_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, sqlx::FromRow)]
pub struct DnsRecord {
    pub id: i64,
    pub claim_id: i64,
    pub fqdn: String,
    pub rtype: String,
    pub content: String,
    pub netbird_zone_id: String,
    pub netbird_id: String,
}

const CLAIM_COLS: &str = "id, owner_id, kind, name, netbird_zone_id";
const RECORD_COLS: &str = "id, claim_id, fqdn, rtype, content, netbird_zone_id, netbird_id";

// ---- claims ----

/// Every claim, any owner. Input for the overlap check.
pub async fn all_claims(db: &SqlitePool) -> sqlx::Result<Vec<DnsClaim>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {CLAIM_COLS} from dns_claims order by name"
    )))
    .fetch_all(db)
    .await
}

/// The claims owned by one user.
pub async fn claims_of(db: &SqlitePool, owner: i64) -> sqlx::Result<Vec<DnsClaim>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {CLAIM_COLS} from dns_claims where owner_id = ? order by name"
    )))
    .bind(owner)
    .fetch_all(db)
    .await
}

/// One claim by id, or `None`.
pub async fn claim_by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<DnsClaim>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {CLAIM_COLS} from dns_claims where id = ?"
    )))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// A unique violation (see `db::is_unique_violation`) means someone else got the name first.
pub async fn insert_claim(
    db: &SqlitePool,
    owner: i64,
    kind: &str,
    name: &str,
    netbird_zone_id: Option<&str>,
) -> sqlx::Result<DnsClaim> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into dns_claims (owner_id, kind, name, netbird_zone_id) values (?, ?, ?, ?) returning {CLAIM_COLS}"
    )))
    .bind(owner)
    .bind(kind)
    .bind(name)
    .bind(netbird_zone_id)
    .fetch_one(db)
    .await
}

/// Also removes the claim's records (ON DELETE CASCADE).
pub async fn delete_claim(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("delete from dns_claims where id = ?")
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

// ---- records ----

/// The records beneath one claim.
pub async fn records_of_claim(db: &SqlitePool, claim: i64) -> sqlx::Result<Vec<DnsRecord>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {RECORD_COLS} from dns_records where claim_id = ? order by fqdn"
    )))
    .bind(claim)
    .fetch_all(db)
    .await
}

/// All records under claims owned by `owner`.
pub async fn records_of_owner(db: &SqlitePool, owner: i64) -> sqlx::Result<Vec<DnsRecord>> {
    sqlx::query_as(
        "select r.id, r.claim_id, r.fqdn, r.rtype, r.content, r.netbird_zone_id, r.netbird_id
         from dns_records r join dns_claims c on c.id = r.claim_id
         where c.owner_id = ? order by r.fqdn",
    )
    .bind(owner)
    .fetch_all(db)
    .await
}

/// One record by id, or `None`.
pub async fn record_by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<DnsRecord>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {RECORD_COLS} from dns_records where id = ?"
    )))
    .bind(id)
    .fetch_optional(db)
    .await
}

/// The record with this exact name, or `None`. Names are unique across all owners.
pub async fn record_by_fqdn(db: &SqlitePool, fqdn: &str) -> sqlx::Result<Option<DnsRecord>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select {RECORD_COLS} from dns_records where fqdn = ?"
    )))
    .bind(fqdn)
    .fetch_optional(db)
    .await
}

/// Remember a record we just created in NetBird: `netbird_zone_id` and `netbird_id` say where it
/// lives there. Fails with a unique violation if the name already has a record.
pub async fn insert_record(
    db: &SqlitePool,
    claim: i64,
    fqdn: &str,
    rtype: &str,
    content: &str,
    netbird_zone_id: &str,
    netbird_id: &str,
) -> sqlx::Result<DnsRecord> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "insert into dns_records (claim_id, fqdn, rtype, content, netbird_zone_id, netbird_id)
         values (?, ?, ?, ?, ?, ?) returning {RECORD_COLS}"
    )))
    .bind(claim)
    .bind(fqdn)
    .bind(rtype)
    .bind(content)
    .bind(netbird_zone_id)
    .bind(netbird_id)
    .fetch_one(db)
    .await
}

/// Local mirror of a NetBird-side record update.
pub async fn update_record_content(
    db: &SqlitePool,
    id: i64,
    rtype: &str,
    content: &str,
) -> sqlx::Result<()> {
    sqlx::query("update dns_records set rtype = ?, content = ? where id = ?")
        .bind(rtype)
        .bind(content)
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

/// Forget a record (after deleting it in NetBird).
pub async fn delete_record(db: &SqlitePool, id: i64) -> sqlx::Result<()> {
    sqlx::query("delete from dns_records where id = ?")
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
    async fn insert_claim_returns_the_row() {
        let (db, a, _) = setup().await;
        let c = insert_claim(&db, a, "tld", "corp", Some("zone-1"))
            .await
            .unwrap();
        assert_eq!(
            (
                c.owner_id,
                c.kind.as_str(),
                c.name.as_str(),
                c.netbird_zone_id.as_deref()
            ),
            (a, "tld", "corp", Some("zone-1"))
        );
        let r = insert_claim(&db, a, "reef", "foo.reef", None)
            .await
            .unwrap();
        assert_eq!(r.netbird_zone_id, None);
        assert_eq!(claim_by_id(&db, c.id).await.unwrap().unwrap(), c);
        assert!(claim_by_id(&db, 999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn duplicate_name_is_a_unique_violation_even_for_another_owner() {
        let (db, a, b) = setup().await;
        insert_claim(&db, a, "tld", "corp", Some("z1"))
            .await
            .unwrap();
        let e = insert_claim(&db, b, "tld", "corp", Some("z2"))
            .await
            .unwrap_err();
        assert!(is_unique_violation(&e), "{e}");
    }

    #[tokio::test]
    async fn all_claims_spans_owners_and_claims_of_filters() {
        let (db, a, b) = setup().await;
        insert_claim(&db, a, "tld", "zeta", Some("z"))
            .await
            .unwrap();
        insert_claim(&db, a, "reef", "alpha.reef", None)
            .await
            .unwrap();
        insert_claim(&db, b, "tld", "beta", Some("y"))
            .await
            .unwrap();

        let all: Vec<_> = all_claims(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(all, ["alpha.reef", "beta", "zeta"]);
        let mine: Vec<_> = claims_of(&db, a)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(mine, ["alpha.reef", "zeta"]);
        assert!(claims_of(&db, 999).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_claim_removes_it_and_its_records_only() {
        let (db, a, _) = setup().await;
        let c1 = insert_claim(&db, a, "tld", "one", Some("z1"))
            .await
            .unwrap();
        let c2 = insert_claim(&db, a, "tld", "two", Some("z2"))
            .await
            .unwrap();
        insert_record(&db, c1.id, "a.one", "A", "1.1.1.1", "z1", "r1")
            .await
            .unwrap();
        insert_record(&db, c2.id, "a.two", "A", "2.2.2.2", "z2", "r2")
            .await
            .unwrap();

        delete_claim(&db, c1.id).await.unwrap();

        assert!(claim_by_id(&db, c1.id).await.unwrap().is_none());
        assert!(records_of_claim(&db, c1.id).await.unwrap().is_empty());
        assert_eq!(records_of_claim(&db, c2.id).await.unwrap().len(), 1);
        delete_claim(&db, 999).await.unwrap(); // missing is not an error
    }

    #[tokio::test]
    async fn record_crud() {
        let (db, a, _) = setup().await;
        let c = insert_claim(&db, a, "tld", "corp", Some("z"))
            .await
            .unwrap();
        let r = insert_record(&db, c.id, "web.corp", "A", "1.1.1.1", "z", "nb-1")
            .await
            .unwrap();
        assert_eq!(
            (r.claim_id, r.fqdn.as_str(), r.netbird_id.as_str()),
            (c.id, "web.corp", "nb-1")
        );

        assert_eq!(record_by_id(&db, r.id).await.unwrap().unwrap(), r);
        assert_eq!(record_by_fqdn(&db, "web.corp").await.unwrap().unwrap(), r);
        assert!(record_by_fqdn(&db, "nope.corp").await.unwrap().is_none());

        update_record_content(&db, r.id, "AAAA", "::1")
            .await
            .unwrap();
        let u = record_by_id(&db, r.id).await.unwrap().unwrap();
        assert_eq!(
            (u.rtype.as_str(), u.content.as_str(), u.netbird_id.as_str()),
            ("AAAA", "::1", "nb-1")
        );

        delete_record(&db, r.id).await.unwrap();
        assert!(record_by_id(&db, r.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn duplicate_fqdn_is_a_unique_violation() {
        let (db, a, _) = setup().await;
        let c = insert_claim(&db, a, "tld", "corp", Some("z"))
            .await
            .unwrap();
        insert_record(&db, c.id, "web.corp", "A", "1.1.1.1", "z", "n1")
            .await
            .unwrap();
        let e = insert_record(&db, c.id, "web.corp", "A", "2.2.2.2", "z", "n2")
            .await
            .unwrap_err();
        assert!(is_unique_violation(&e));
    }

    #[tokio::test]
    async fn records_of_owner_only_returns_that_owners_records() {
        let (db, a, b) = setup().await;
        let ca = insert_claim(&db, a, "tld", "aa", Some("z1")).await.unwrap();
        let cb = insert_claim(&db, b, "reef", "bb.reef", None).await.unwrap();
        insert_record(&db, ca.id, "y.aa", "A", "1.1.1.1", "z1", "r1")
            .await
            .unwrap();
        insert_record(&db, ca.id, "x.aa", "A", "1.1.1.2", "z1", "r2")
            .await
            .unwrap();
        insert_record(&db, cb.id, "w.bb.reef", "A", "1.1.1.3", "zr", "r3")
            .await
            .unwrap();

        let mine: Vec<_> = records_of_owner(&db, a)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.fqdn)
            .collect();
        assert_eq!(mine, ["x.aa", "y.aa"]);
        assert_eq!(records_of_owner(&db, b).await.unwrap().len(), 1);
    }
}
