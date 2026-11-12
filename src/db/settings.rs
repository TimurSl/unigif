//! Server-wide key/value settings.

use super::{now_ms, Db};

pub async fn get(db: &Db, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?").bind(key).fetch_optional(db).await
}

pub async fn set(db: &Db, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now_ms())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete(db: &Db, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM settings WHERE key = ?").bind(key).execute(db).await?;
    Ok(())
}
