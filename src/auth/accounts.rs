//! Creating owners and setting passwords, shared by the CLI and start-up.

use anyhow::{bail, Context, Result};

use super::password;
use crate::config::Config;
use crate::db::{sessions, users, Db};

pub async fn create_user(db: &Db, username: &str, password: &str) -> Result<i64> {
    let username = username.trim();
    if let Err(e) = users::valid_username(username) {
        bail!("{e}");
    }
    if let Err(e) = password::validate_new(password) {
        bail!("{e}");
    }
    if users::find_by_username(db, username).await?.is_some() {
        bail!("user {username:?} already exists");
    }
    let phc = hash(password).await?;
    users::create(db, username, &phc).await.context("creating user")
}

/// Set a new password and log the user out everywhere.
pub async fn set_password(db: &Db, username: &str, password: &str) -> Result<()> {
    if let Err(e) = password::validate_new(password) {
        bail!("{e}");
    }
    let user = users::find_by_username(db, username.trim())
        .await?
        .with_context(|| format!("no such user {username:?}"))?;
    let phc = hash(password).await?;
    users::set_password(db, user.id, &phc).await?;
    sessions::delete_for_user(db, user.id, None).await?;
    Ok(())
}

async fn hash(password: &str) -> Result<String> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || password::hash(&password)).await?
}

/// `auth.initialOwner`: create the first user when there is none yet.
pub async fn ensure_initial_owner(cfg: &Config, db: &Db) -> Result<()> {
    let owner = &cfg.auth.initial_owner;
    let (Some(name), Some(file)) = (owner.username.as_deref(), owner.password_file.as_deref()) else {
        return Ok(());
    };
    if users::count(db).await? > 0 {
        return Ok(());
    }
    let raw = tokio::fs::read_to_string(file).await.with_context(|| format!("reading {}", file.display()))?;
    let password = raw.lines().next().unwrap_or("");
    create_user(db, name, password).await.context("creating the initial owner")?;
    tracing::info!(username = name, "created the initial owner");
    Ok(())
}
