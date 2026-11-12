//! SQLite pool setup and migrations.

use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

pub mod gifs;
pub mod keys;
pub mod sessions;
pub mod settings;
pub mod users;

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
    let backups = path.parent().map(crate::backup::default_dir);
    connect(opts, backups.as_deref()).await
}

/// Start a transaction that will write.
///
/// SQLite's default `BEGIN` is deferred: a transaction that reads first and
/// writes later fails immediately with `SQLITE_BUSY_SNAPSHOT` if another
/// connection committed in between, and `busy_timeout` does not help. Taking
/// the write lock up front makes concurrent writers queue instead.
pub async fn begin_write(db: &Db) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
    db.begin_with("BEGIN IMMEDIATE").await
}

/// In-memory database for tests (single connection, so the DB is shared).
pub async fn open_memory() -> Result<Db> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await.context("running migrations")?;
    Ok(pool)
}

async fn connect(opts: SqliteConnectOptions, backups: Option<&std::path::Path>) -> Result<Db> {
    let pool =
        SqlitePoolOptions::new().max_connections(8).connect_with(opts).await.context("opening database")?;
    let migrator = sqlx::migrate!("./migrations");
    if let Some(dir) = backups {
        // An existing database that is about to change shape gets a copy first.
        // Failing to make it stops the start: an upgrade without a way back
        // is not worth the risk.
        if let Some(applied) = latest_applied_migration(&pool).await? {
            if migrator.iter().any(|m| m.version > applied) {
                let dest = crate::backup::before_migration(&pool, dir, applied)
                    .await
                    .context("snapshotting the database before migrating it")?;
                tracing::info!("database snapshot before migration: {}", dest.display());
            }
        }
    }
    migrator.run(&pool).await.context("running migrations")?;
    Ok(pool)
}

/// The newest migration already applied, or `None` for a database that has
/// never been migrated (a new file).
async fn latest_applied_migration(pool: &Db) -> Result<Option<i64>> {
    let has_table: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await?;
    if has_table == 0 {
        return Ok(None);
    }
    Ok(sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success = 1")
        .fetch_one(pool)
        .await?)
}
