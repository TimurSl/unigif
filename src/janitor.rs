//! Housekeeping: purge old trash, drop expired sessions, trim the cache.

use std::time::Duration;

use anyhow::Result;

use crate::app::App;
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
    for id in &ids {
        // Files first: a crash in between leaves a row that is purged next time.
        let _ = tokio::fs::remove_dir_all(media::gif_dir(&app.cfg.media_dir(), id)).await;
        crate::cache::remove_gif(app, id).await;
        let _ = tokio::fs::remove_file(crate::ingest::worker::source_path(&app.cfg, id)).await;
        let mut tx = crate::db::begin_write(&app.db).await?;
        // Trashed GIFs no longer count in tags.gif_count, so only the rows go.
        sqlx::query("DELETE FROM jobs WHERE gif_id = ?").bind(id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM gifs WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    if !ids.is_empty() {
        // Hashtags that only lived on purged GIFs.
        sqlx::query("DELETE FROM tags WHERE NOT EXISTS (SELECT 1 FROM gif_tags WHERE tag_id = tags.id)")
            .execute(&app.db)
            .await?;
    }
    Ok(ids.len())
}

pub async fn run_once(app: &App) -> Result<()> {
    let now = now_ms();
    let purged = purge_trash(app, now).await?;
    let sessions = sessions::purge_expired(&app.db, now).await?;
    let evicted = crate::cache::evict(app, None).await?;
    if purged > 0 || sessions > 0 || evicted > 0 {
        tracing::info!(
            purged_gifs = purged,
            expired_sessions = sessions,
            evicted_cache_files = evicted,
            "janitor"
        );
    }
    Ok(())
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
