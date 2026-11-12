//! API key rows.

use sqlx::FromRow;

use super::Db;

#[derive(Debug, Clone, FromRow)]
pub struct KeyRow {
    pub id: String,
    pub name: String,
    pub secret_hash: Vec<u8>,
    pub scopes: String,
    pub deny_tags: String,
    pub rate_limit: Option<i64>,
    pub created_by: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl KeyRow {
    pub fn deny_list(&self) -> Vec<String> {
        serde_json::from_str(&self.deny_tags).unwrap_or_default()
    }

    pub fn is_usable(&self, now: i64) -> bool {
        self.revoked_at.is_none() && self.expires_at.map_or(true, |e| e > now)
    }
}

pub async fn insert(db: &Db, k: &KeyRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO api_keys (id, name, secret_hash, scopes, deny_tags, rate_limit, created_by, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&k.id)
    .bind(&k.name)
    .bind(&k.secret_hash)
    .bind(&k.scopes)
    .bind(&k.deny_tags)
    .bind(k.rate_limit)
    .bind(k.created_by)
    .bind(k.created_at)
    .bind(k.expires_at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn get(db: &Db, id: &str) -> Result<Option<KeyRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM api_keys WHERE id = ?").bind(id).fetch_optional(db).await
}

pub async fn list(db: &Db, include_revoked: bool) -> Result<Vec<KeyRow>, sqlx::Error> {
    let sql = if include_revoked {
        "SELECT * FROM api_keys ORDER BY created_at DESC, id"
    } else {
        "SELECT * FROM api_keys WHERE revoked_at IS NULL ORDER BY created_at DESC, id"
    };
    sqlx::query_as(sql).fetch_all(db).await
}

pub async fn update(db: &Db, k: &KeyRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE api_keys SET name = ?, scopes = ?, deny_tags = ?, rate_limit = ?, expires_at = ? WHERE id = ?",
    )
    .bind(&k.name)
    .bind(&k.scopes)
    .bind(&k.deny_tags)
    .bind(k.rate_limit)
    .bind(k.expires_at)
    .bind(&k.id)
    .execute(db)
    .await?;
    Ok(())
}

/// Returns whether a live key was revoked.
pub async fn revoke(db: &Db, id: &str, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE api_keys SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(db)
        .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn touch_last_used(db: &Db, id: &str, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE id = ?").bind(now).bind(id).execute(db).await?;
    Ok(())
}
