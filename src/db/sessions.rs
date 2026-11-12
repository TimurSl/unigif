//! Server-side login sessions. Only `sha256(cookie value)` is stored.

use sqlx::FromRow;

use super::Db;

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub token_hash: Vec<u8>,
    pub user_id: i64,
    pub username: String,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub expires_at: i64,
}

impl SessionRow {
    /// Public handle for listing and revoking: the first 8 bytes of the hash, hex.
    pub fn public_id(&self) -> String {
        public_id(&self.token_hash)
    }
}

pub fn public_id(token_hash: &[u8]) -> String {
    token_hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub async fn create(
    db: &Db,
    token_hash: &[u8],
    user_id: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
    now: i64,
    expires_at: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, user_agent, ip, created_at, last_seen_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(user_agent)
    .bind(ip)
    .bind(now)
    .bind(now)
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(())
}

const SELECT: &str = "SELECT s.token_hash, s.user_id, u.username, s.user_agent, s.ip,
                             s.created_at, s.last_seen_at, s.expires_at
                      FROM sessions s JOIN users u ON u.id = s.user_id";

pub async fn find_valid(db: &Db, token_hash: &[u8], now: i64) -> Result<Option<SessionRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE s.token_hash = ? AND s.expires_at > ?"))
        .bind(token_hash)
        .bind(now)
        .fetch_optional(db)
        .await
}

pub async fn touch(db: &Db, token_hash: &[u8], now: i64, expires_at: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ? WHERE token_hash = ?")
        .bind(now)
        .bind(expires_at)
        .bind(token_hash)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete(db: &Db, token_hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(token_hash).execute(db).await?;
    Ok(())
}

pub async fn list(db: &Db, user_id: i64, now: i64) -> Result<Vec<SessionRow>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE s.user_id = ? AND s.expires_at > ? ORDER BY s.last_seen_at DESC"))
        .bind(user_id)
        .bind(now)
        .fetch_all(db)
        .await
}

/// Revoke by public id; returns whether a session was removed.
pub async fn delete_by_public_id(db: &Db, user_id: i64, id: &str) -> Result<bool, sqlx::Error> {
    let rows = list(db, user_id, 0).await?;
    let Some(row) = rows.into_iter().find(|r| r.public_id() == id) else { return Ok(false) };
    delete(db, &row.token_hash).await?;
    Ok(true)
}

/// Remove every session of a user, optionally keeping one.
pub async fn delete_for_user(db: &Db, user_id: i64, keep: Option<&[u8]>) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND (? IS NULL OR token_hash != ?)")
        .bind(user_id)
        .bind(keep)
        .bind(keep)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn purge_expired(db: &Db, now: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM sessions WHERE expires_at <= ?").bind(now).execute(db).await?.rows_affected())
}
