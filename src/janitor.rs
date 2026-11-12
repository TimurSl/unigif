//! Housekeeping: purge old trash, drop expired sessions, trim the cache.

use std::time::Duration;

use anyhow::Result;

use crate::app::App;
use crate::db::gifs::Visibility;
use crate::db::{now_ms, sessions};
use crate::media;

/// Delete GIFs trashed longer ago than `trash.retentionDays`, files included.
/// Returns how many were purged.
pub async fn purge_trash(app: &App, now: i64) -> Result<usize> {
    let cutoff = now - app.cfg.trash.retention_days as i64 * 86_400_000;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM gifs WHERE deleted_at IS NOT NULL AND deleted_at <= ?")
            .bind(cutoff)
            .fetch_all(&app.db)
            .await?;
    purge(app, &ids).await
}

/// Empty the trash now: every trashed GIF the credential can see, regardless
/// of age. Returns how many were purged.
pub async fn empty_trash(app: &App, vis: &Visibility) -> Result<usize> {
    let mut qb = sqlx::QueryBuilder::new("SELECT g.id FROM gifs g WHERE g.deleted_at IS NOT NULL");
    vis.push_clause(&mut qb, "g");
    let ids: Vec<String> = qb.build_query_scalar().fetch_all(&app.db).await?;
    purge(app, &ids).await
}

/// Delete trashed GIFs for good: stored media, cached conversions, any
/// leftover source, and the rows. GIFs that were restored in the meantime
/// are left alone. Returns how many were purged.
pub async fn purge(app: &App, ids: &[String]) -> Result<usize> {
    let mut purged = 0;
    for id in ids {
        // Re-check under the write lock that it is still in the trash, so a
        // restore that raced the purge wins instead of losing its files.
        let mut tx = crate::db::begin_write(&app.db).await?;
        let trashed: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM gifs WHERE id = ? AND deleted_at IS NOT NULL")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if trashed.is_none() {
            continue;
        }
        // Files while the write lock is held: a restore cannot slip in
        // between, and a crash before the commit leaves a trashed row that is
        // purged next time. Variant rows go with the GIF (ON DELETE CASCADE).
        let _ = tokio::fs::remove_dir_all(media::gif_dir(&app.cfg.media_dir(), id)).await;
        let _ = tokio::fs::remove_dir_all(app.cfg.cache_gif_dir(id)).await;
        let _ = tokio::fs::remove_file(crate::ingest::worker::source_path(&app.cfg, id)).await;
        // Trashed GIFs no longer count in tags.gif_count, so only the rows go.
        sqlx::query("DELETE FROM jobs WHERE gif_id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM gifs WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        purged += 1;
    }
    if purged > 0 {
        // Hashtags that only lived on purged GIFs.
        sqlx::query("DELETE FROM tags WHERE NOT EXISTS (SELECT 1 FROM gif_tags WHERE tag_id = tags.id)")
            .execute(&app.db)
            .await?;
    }
    Ok(purged)
}

/// What one janitor pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub purged_gifs: usize,
    pub expired_sessions: u64,
    pub evicted_files: usize,
    pub evicted_bytes: u64,
}

/// One pass: old trash, expired sessions, the cache trimmed to its limit.
/// The hourly loop and `unigif gc` both run this.
pub async fn run_once(app: &App) -> Result<Report> {
    let now = now_ms();
    let purged_gifs = purge_trash(app, now).await?;
    let expired_sessions = sessions::purge_expired(&app.db, now).await?;
    let (evicted_files, evicted_bytes) = crate::cache::evict(app, None).await?;
    let r = Report { purged_gifs, expired_sessions, evicted_files, evicted_bytes };
    if r != Report::default() {
        tracing::info!(
            purged_gifs = r.purged_gifs,
            expired_sessions = r.expired_sessions,
            evicted_cache_files = r.evicted_files,
            evicted_cache_bytes = r.evicted_bytes,
            "janitor"
        );
    }
    Ok(r)
}

/// Run hourly until the process ends.
pub fn spawn(app: &App) -> tokio::task::JoinHandle<()> {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_once(&app).await {
                tracing::warn!(error = ?e, "janitor failed");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    })
}
