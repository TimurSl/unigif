//! SQLite pool setup and migrations.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub mod gifs;

pub type Db = SqlitePool;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Open (creating if needed) the database file and run migrations.
pub async fn open(path: &std::path::Path) -> Result<Db> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_context(|| format!("creating {parent:?}"))?;
    }
    let url = format!("sqlite://{}", path.display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    connect(opts).await
}

/// In-memory database for tests (single connection, so the DB is shared).
pub async fn open_memory() -> Result<Db> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await.context("running migrations")?;
    Ok(pool)
}

async fn connect(opts: SqliteConnectOptions) -> Result<Db> {
    let pool =
        SqlitePoolOptions::new().max_connections(8).connect_with(opts).await.context("opening database")?;
    sqlx::migrate!("./migrations").run(&pool).await.context("running migrations")?;
    Ok(pool)
}
