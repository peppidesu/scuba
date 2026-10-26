//! State shared by every handler (axum's `State` extractor).

use sqlx::SqlitePool;

use crate::netbird::NetbirdClient;

/// Cheap to clone: the pool and the client are both handles to shared resources.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub netbird: NetbirdClient,
}
