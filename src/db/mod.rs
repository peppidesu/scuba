//! Typed SQL, one module per table group. No business rules live here: the rules are in
//! `policy`, and `services` decides when to call what.
//!
//! Conventions: every function takes `&SqlitePool` first, and rows come back as plain structs.
//! Inserts that can race on a unique column (claim names, group names) just return the
//! database error; callers use [`is_unique_violation`] to turn it into a friendly message.

use std::str::FromStr;

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

pub mod dns;
pub mod groups;
pub mod sessions;
pub mod users;

/// Open (creating if needed) the sqlite database at `url` and bring its schema up to date.
///
/// WAL mode lets reads proceed during writes, and foreign keys are switched on because sqlite
/// ignores them by default — the schema relies on `ON DELETE CASCADE`.
/// `sqlite::memory:` gives an in-memory database, which the tests use.
pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

/// True when an insert/update was rejected by a UNIQUE constraint, e.g. someone else claimed
/// the name between our check and our insert.
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}
