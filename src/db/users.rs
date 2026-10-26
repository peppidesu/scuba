//! People who have logged in. A row is created the first time someone logs in through
//! authentik and refreshed on every login after that.

use sqlx::SqlitePool;

/// One person. Identified by `oidc_sub`, authentik's stable id for them.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    /// authentik's `sub` claim. Never changes, unlike email or username.
    pub oidc_sub: String,
    pub email: String,
    pub display_name: String,
    /// Their NetBird user, which is how we find their devices. Empty until linked
    /// (planned: match on email); until then the user has no devices in scuba.
    pub netbird_user_id: Option<String>,
}

/// Find the user with this `sub`, creating them if this is their first login.
/// Email and name are updated every time, so changes in authentik show up here.
pub async fn upsert_by_sub(
    db: &SqlitePool,
    sub: &str,
    email: &str,
    name: &str,
) -> sqlx::Result<User> {
    sqlx::query_as(
        "insert into users (oidc_sub, email, display_name) values (?, ?, ?)
         on conflict(oidc_sub) do 
            update set email = excluded.email, display_name = excluded.display_name
         returning 
            id, oidc_sub, email, display_name, netbird_user_id",
    )
    .bind(sub)
    .bind(email)
    .bind(name)
    .fetch_one(db)
    .await
}

/// Look a user up by our own id (what the session stores).
pub async fn by_id(db: &SqlitePool, id: i64) -> sqlx::Result<Option<User>> {
    sqlx::query_as(
        "select id, oidc_sub, email, display_name, netbird_user_id from users 
         where id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        crate::db::connect("sqlite::memory:").await.unwrap()
    }

    #[tokio::test]
    async fn upsert_creates_a_user() {
        let db = pool().await;
        let u = upsert_by_sub(&db, "sub-1", "a@example.com", "alice")
            .await
            .unwrap();
        assert_eq!(
            (
                u.oidc_sub.as_str(),
                u.email.as_str(),
                u.display_name.as_str()
            ),
            ("sub-1", "a@example.com", "alice")
        );
        assert_eq!(u.netbird_user_id, None);
    }

    #[tokio::test]
    async fn upsert_is_keyed_on_sub_and_refreshes_profile() {
        let db = pool().await;
        let first = upsert_by_sub(&db, "sub-1", "a@example.com", "alice")
            .await
            .unwrap();
        let second = upsert_by_sub(&db, "sub-1", "new@example.com", "alice2")
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(
            (second.email.as_str(), second.display_name.as_str()),
            ("new@example.com", "alice2")
        );
        let (count,): (i64,) = sqlx::query_as("select count(*) from users")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn upsert_keeps_netbird_link() {
        let db = pool().await;
        let u = upsert_by_sub(&db, "sub-1", "a@example.com", "alice")
            .await
            .unwrap();
        sqlx::query("update users set netbird_user_id = 'nb-1' where id = ?")
            .bind(u.id)
            .execute(&db)
            .await
            .unwrap();
        let again = upsert_by_sub(&db, "sub-1", "a@example.com", "alice")
            .await
            .unwrap();
        assert_eq!(again.netbird_user_id.as_deref(), Some("nb-1"));
    }

    #[tokio::test]
    async fn different_subs_are_different_users() {
        let db = pool().await;
        let a = upsert_by_sub(&db, "sub-a", "same@example.com", "a")
            .await
            .unwrap();
        let b = upsert_by_sub(&db, "sub-b", "same@example.com", "b")
            .await
            .unwrap();
        assert_ne!(a.id, b.id);
    }

    #[tokio::test]
    async fn by_id_finds_or_returns_none() {
        let db = pool().await;
        let u = upsert_by_sub(&db, "sub-1", "a@example.com", "alice")
            .await
            .unwrap();
        assert_eq!(by_id(&db, u.id).await.unwrap().unwrap().oidc_sub, "sub-1");
        assert!(by_id(&db, u.id + 1000).await.unwrap().is_none());
    }
}
