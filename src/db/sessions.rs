//! Session persistence: plain queries over the shared pool, like the other `db` modules.
//! `auth::sessions` adapts them to tower-sessions.
//!
//! One row per session: `id` is the (random) session id, `data` the session contents as JSON,
//! `expiry` a unix timestamp in seconds. Expired rows are ignored when reading and deleted
//! periodically by [`cleanup_loop`].
//!
//! (We write this ourselves because `tower-sessions-sqlx-store` is built on sqlx 0.8, which
//! can't be used alongside sqlx 0.9.)

use sqlx::SqlitePool;
use tower_sessions::cookie::time::OffsetDateTime;

/// The current time as a unix timestamp in seconds, the unit of the `expiry` column.
fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

/// Add a new session. Returns `false`, without writing, if a session with this id already exists.
pub async fn insert(db: &SqlitePool, id: &str, data: &str, expiry: i64) -> sqlx::Result<bool> {
    match sqlx::query("insert into sessions (id, data, expiry) values (?, ?, ?)")
        .bind(id)
        .bind(data)
        .bind(expiry)
        .execute(db)
        .await
    {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
        Err(e) => Err(e),
    }
}

/// Create the session, or overwrite its data and expiry if it already exists.
pub async fn upsert(db: &SqlitePool, id: &str, data: &str, expiry: i64) -> sqlx::Result<()> {
    sqlx::query(
        "insert into sessions (id, data, expiry) values (?, ?, ?)
         on conflict(id) do update set data = excluded.data, expiry = excluded.expiry",
    )
    .bind(id)
    .bind(data)
    .bind(expiry)
    .execute(db)
    .await
    .map(drop)
}

/// The session's JSON data, or `None` if there is no such session or it has expired.
pub async fn load(db: &SqlitePool, id: &str) -> sqlx::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("select data from sessions where id = ? and expiry > ?")
            .bind(id)
            .bind(now())
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(d,)| d))
}

/// Remove one session (logout). Not an error if it is already gone.
pub async fn delete(db: &SqlitePool, id: &str) -> sqlx::Result<()> {
    sqlx::query("delete from sessions where id = ?")
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

/// Remove every expired session.
pub async fn delete_expired(db: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query("delete from sessions where expiry <= ?")
        .bind(now())
        .execute(db)
        .await
        .map(drop)
}

/// Delete expired sessions every `every`, forever. Run it once as a background task at startup;
/// failures are logged and retried at the next tick.
pub async fn cleanup_loop(db: SqlitePool, every: std::time::Duration) {
    let mut tick = tokio::time::interval(every);
    loop {
        tick.tick().await;
        if let Err(e) = delete_expired(&db).await {
            tracing::warn!("session cleanup failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        crate::db::connect("sqlite::memory:").await.unwrap()
    }

    fn future() -> i64 {
        now() + 3600
    }

    #[tokio::test]
    async fn insert_then_load() {
        let db = pool().await;
        assert!(insert(&db, "s1", "{\"a\":1}", future()).await.unwrap());
        assert_eq!(load(&db, "s1").await.unwrap().as_deref(), Some("{\"a\":1}"));
    }

    #[tokio::test]
    async fn insert_reports_id_collision_without_overwriting() {
        let db = pool().await;
        assert!(insert(&db, "s1", "first", future()).await.unwrap());
        assert!(!insert(&db, "s1", "second", future()).await.unwrap());
        assert_eq!(load(&db, "s1").await.unwrap().as_deref(), Some("first"));
    }

    #[tokio::test]
    async fn upsert_inserts_and_overwrites() {
        let db = pool().await;
        upsert(&db, "s1", "one", future()).await.unwrap();
        upsert(&db, "s1", "two", future()).await.unwrap();
        assert_eq!(load(&db, "s1").await.unwrap().as_deref(), Some("two"));
    }

    #[tokio::test]
    async fn upsert_can_extend_expiry() {
        let db = pool().await;
        upsert(&db, "s1", "x", now() - 10).await.unwrap();
        assert!(load(&db, "s1").await.unwrap().is_none());
        upsert(&db, "s1", "x", future()).await.unwrap();
        assert!(load(&db, "s1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn load_ignores_expired_and_unknown() {
        let db = pool().await;
        insert(&db, "old", "x", now() - 1).await.unwrap();
        assert!(load(&db, "old").await.unwrap().is_none());
        assert!(load(&db, "nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_only_that_session() {
        let db = pool().await;
        insert(&db, "a", "x", future()).await.unwrap();
        insert(&db, "b", "x", future()).await.unwrap();
        delete(&db, "a").await.unwrap();
        assert!(load(&db, "a").await.unwrap().is_none());
        assert!(load(&db, "b").await.unwrap().is_some());
        delete(&db, "missing").await.unwrap(); // not an error
    }

    #[tokio::test]
    async fn delete_expired_removes_only_expired_rows() {
        let db = pool().await;
        insert(&db, "old1", "x", now() - 100).await.unwrap();
        insert(&db, "old2", "x", now() - 1).await.unwrap();
        insert(&db, "live", "x", future()).await.unwrap();
        delete_expired(&db).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("select count(*) from sessions")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert!(load(&db, "live").await.unwrap().is_some());
    }
}
