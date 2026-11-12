//! Owner accounts.

use sqlx::FromRow;

use super::{now_ms, Db};

#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub prefs: String,
    pub created_at: i64,
}

pub fn valid_username(name: &str) -> Result<(), &'static str> {
    let n = name.chars().count();
    if n == 0 || n > 64 {
        return Err("username must be 1 to 64 characters");
    }
    if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("username may not contain whitespace or control characters");
    }
    Ok(())
}

pub async fn count(db: &Db) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(db).await
}

pub async fn create(db: &Db, username: &str, password_hash: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, created_at) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(username)
    .bind(password_hash)
    .bind(now_ms())
    .fetch_one(db)
    .await
}

pub async fn find_by_username(db: &Db, username: &str) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM users WHERE username = ?").bind(username).fetch_optional(db).await
}

pub async fn find(db: &Db, id: i64) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM users WHERE id = ?").bind(id).fetch_optional(db).await
}

/// The earliest account: the owner that CLI-minted keys belong to.
pub async fn first(db: &Db) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM users ORDER BY id LIMIT 1").fetch_optional(db).await
}

pub async fn set_password(db: &Db, id: i64, password_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn set_prefs(db: &Db, id: i64, prefs: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE users SET prefs = ? WHERE id = ?").bind(prefs).bind(id).execute(db).await?;
    Ok(())
}
