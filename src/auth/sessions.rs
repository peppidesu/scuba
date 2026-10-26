//! The bridge between tower-sessions and our database.
//!
//! tower-sessions wants something implementing [`SessionStore`]; this implements it by
//! calling the plain queries in [`crate::db::sessions`], so session storage uses the same
//! connection pool as everything else.

use async_trait::async_trait;
use sqlx::SqlitePool;
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store::{Error, Result},
};

use crate::db::sessions as queries;

/// Stores sessions in the `sessions` table.
#[derive(Clone, Debug)]
pub struct PoolSessionStore(pub SqlitePool);

fn backend_error(e: sqlx::Error) -> Error {
    Error::Backend(e.to_string())
}

/// A whole session record as the JSON text we keep in the `data` column.
fn encode(record: &Record) -> Result<String> {
    serde_json::to_string(record).map_err(|e| Error::Encode(e.to_string()))
}

#[async_trait]
impl SessionStore for PoolSessionStore {
    async fn create(&self, record: &mut Record) -> Result<()> {
        // Session ids are random, so a collision is astronomically unlikely, but the contract
        // says to handle it: `insert` reports one as `false`, and we try again with a new id.
        loop {
            let inserted = queries::insert(
                &self.0,
                &record.id.to_string(),
                &encode(record)?,
                record.expiry_date.unix_timestamp(),
            )
            .await
            .map_err(backend_error)?;
            if inserted {
                return Ok(());
            }
            record.id = Id::default();
        }
    }

    async fn save(&self, record: &Record) -> Result<()> {
        queries::upsert(
            &self.0,
            &record.id.to_string(),
            &encode(record)?,
            record.expiry_date.unix_timestamp(),
        )
        .await
        .map_err(backend_error)
    }

    async fn load(&self, id: &Id) -> Result<Option<Record>> {
        let json = queries::load(&self.0, &id.to_string())
            .await
            .map_err(backend_error)?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| Error::Decode(e.to_string())))
            .transpose()
    }

    async fn delete(&self, id: &Id) -> Result<()> {
        queries::delete(&self.0, &id.to_string())
            .await
            .map_err(backend_error)
    }
}

#[cfg(test)]
mod tests {
    use tower_sessions::cookie::time::{Duration, OffsetDateTime};

    use super::*;

    async fn store() -> PoolSessionStore {
        PoolSessionStore(crate::db::connect("sqlite::memory:").await.unwrap())
    }

    fn record(expires_in: Duration) -> Record {
        let mut data = std::collections::HashMap::new();
        data.insert("user".to_owned(), serde_json::json!({ "n": 7, "s": "x" }));
        Record {
            id: Id::default(),
            data,
            expiry_date: OffsetDateTime::now_utc() + expires_in,
        }
    }

    #[tokio::test]
    async fn create_then_load_round_trips() {
        let s = store().await;
        let mut r = record(Duration::hours(1));
        s.create(&mut r).await.unwrap();
        let loaded = s.load(&r.id).await.unwrap().expect("session exists");
        assert_eq!(loaded.id, r.id);
        assert_eq!(loaded.data, r.data);
    }

    #[tokio::test]
    async fn create_picks_a_new_id_on_collision() {
        let s = store().await;
        let mut a = record(Duration::hours(1));
        s.create(&mut a).await.unwrap();

        let mut b = record(Duration::hours(1));
        b.id = a.id;
        b.data.insert("who".into(), serde_json::json!("b"));
        s.create(&mut b).await.unwrap();

        assert_ne!(a.id, b.id);
        assert_eq!(s.load(&a.id).await.unwrap().unwrap().data, a.data);
        assert_eq!(s.load(&b.id).await.unwrap().unwrap().data, b.data);
    }

    #[tokio::test]
    async fn save_overwrites_existing_and_creates_missing() {
        let s = store().await;
        let mut r = record(Duration::hours(1));
        s.create(&mut r).await.unwrap();
        r.data.insert("extra".into(), serde_json::json!(true));
        s.save(&r).await.unwrap();
        assert_eq!(s.load(&r.id).await.unwrap().unwrap().data, r.data);

        let fresh = record(Duration::hours(1));
        s.save(&fresh).await.unwrap();
        assert!(s.load(&fresh.id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn expired_sessions_do_not_load() {
        let s = store().await;
        let r = record(Duration::seconds(-5));
        s.save(&r).await.unwrap();
        assert!(s.load(&r.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_removes_the_session() {
        let s = store().await;
        let mut r = record(Duration::hours(1));
        s.create(&mut r).await.unwrap();
        s.delete(&r.id).await.unwrap();
        assert!(s.load(&r.id).await.unwrap().is_none());
    }
}
