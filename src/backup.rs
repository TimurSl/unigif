//! Consistent database snapshots: `unigif backup` and the pre-migration copy.
//!
//! A snapshot is made with `VACUUM INTO`, which reads the database inside one
//! read transaction, so it is consistent while the server keeps writing, and
//! it produces a compact file with no WAL beside it. `media/` is immutable and
//! is not copied; back it up with the host's own tool.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use time::OffsetDateTime;

use crate::db::Db;

/// Prefix of the files `unigif backup` writes (and prunes).
pub const BACKUP_PREFIX: &str = "unigif-";
/// Prefix of the copies taken before a migration (and pruned separately).
pub const PRE_MIGRATION_PREFIX: &str = "pre-migration-";
/// How many pre-migration copies are kept.
pub const PRE_MIGRATION_KEEP: usize = 5;

/// `<dataDir>/backups`, the default destination.
pub fn default_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("backups")
}

fn stamp() -> String {
    let t = OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}{:03}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

/// Write a consistent snapshot of `db` to `dest`, which must not exist.
/// The file appears under its final name only once it is complete.
pub async fn snapshot(db: &Db, dest: &Path) -> Result<()> {
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = dest.with_extension("db.partial");
    // A leftover from an interrupted run would make VACUUM INTO fail.
    let _ = tokio::fs::remove_file(&tmp).await;
    let target = tmp.to_str().context("backup path is not valid UTF-8")?.replace('\'', "''");
    let res = sqlx::query(&format!("VACUUM INTO '{target}'")).execute(db).await;
    if let Err(e) = res {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow::Error::new(e).context("writing the snapshot"));
    }
    tokio::fs::rename(&tmp, dest).await.context("moving the snapshot into place")?;
    Ok(())
}

/// Take a timestamped snapshot into `dir` and keep only the newest `keep`.
pub async fn run(db: &Db, dir: &Path, keep: usize) -> Result<PathBuf> {
    if keep == 0 {
        bail!("--keep must be at least 1");
    }
    let dest = dir.join(format!("{BACKUP_PREFIX}{}.db", stamp()));
    snapshot(db, &dest).await?;
    prune(dir, BACKUP_PREFIX, keep).await?;
    Ok(dest)
}

/// Copy the database aside before migrations change it. Called with a pool
/// that has not migrated yet; does nothing for a new or up-to-date database.
pub async fn before_migration(db: &Db, dir: &Path, pending_from: i64) -> Result<PathBuf> {
    let dest = dir.join(format!("{PRE_MIGRATION_PREFIX}v{pending_from}-{}.db", stamp()));
    snapshot(db, &dest).await?;
    prune(dir, PRE_MIGRATION_PREFIX, PRE_MIGRATION_KEEP).await?;
    Ok(dest)
}

/// Delete all but the newest `keep` finished `<prefix>*.db` files in `dir`.
/// The timestamp is in the name, so lexical order is chronological.
async fn prune(dir: &Path, prefix: &str, keep: usize) -> Result<()> {
    let mut names = vec![];
    let mut rd = tokio::fs::read_dir(dir).await.with_context(|| format!("reading {}", dir.display()))?;
    while let Some(e) = rd.next_entry().await? {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) && name.ends_with(".db") {
            names.push(name);
        }
    }
    names.sort();
    let excess = names.len().saturating_sub(keep);
    for name in names.into_iter().take(excess) {
        tokio::fs::remove_file(dir.join(&name)).await.with_context(|| format!("removing {name}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prune_keeps_the_newest_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "unigif-20260101-000000.db",
            "unigif-20260102-000000.db",
            "unigif-20260103-000000.db",
            "unigif-20260104-000000.db.partial",
            "pre-migration-v1-20260101-000000.db",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        prune(dir.path(), BACKUP_PREFIX, 2).await.unwrap();
        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "notes.txt",
                "pre-migration-v1-20260101-000000.db",
                "unigif-20260102-000000.db",
                "unigif-20260103-000000.db",
                "unigif-20260104-000000.db.partial",
            ]
        );
    }
}
