//! In-process job workers. Jobs live in the `jobs` table, so a restart
//! resumes unfinished work.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use sqlx::FromRow;
use tokio::task::JoinHandle;

use crate::app::App;
use crate::config::{Config, KeepOriginals};
use crate::db::gifs;
use crate::db::now_ms;
use crate::media::transcode::{self, Encoded};
use crate::media::{self, SourceKind};

const MAX_ATTEMPTS: i64 = 3;
const IDLE_POLL: Duration = Duration::from_secs(5);

pub fn source_path(cfg: &Config, id: &str) -> PathBuf {
    cfg.work_dir().join(format!("{id}.src"))
}

fn out_path(cfg: &Config, id: &str) -> PathBuf {
    cfg.tmp_dir().join(format!("{id}.out"))
}

#[derive(FromRow)]
struct Job {
    id: i64,
    gif_id: Option<String>,
    attempts: i64,
}

/// Startup housekeeping: interrupted jobs go back in the queue, `tmp/` is
/// wiped, and sources of finished GIFs left behind by a crash are removed.
pub async fn recover(app: &App) -> Result<()> {
    sqlx::query("UPDATE jobs SET state = 'queued' WHERE state = 'running'").execute(&app.db).await?;
    let tmp = app.cfg.tmp_dir();
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    tokio::fs::create_dir_all(app.cfg.work_dir()).await?;

    let mut rd = tokio::fs::read_dir(app.cfg.work_dir()).await?;
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".src") else { continue };
        let keep = match gifs::get(&app.db, id).await? {
            Some(row) => row.status == "processing",
            None => false,
        };
        if !keep {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

/// Start `n` worker loops.
pub fn spawn(app: &App, n: usize) -> Vec<JoinHandle<()>> {
    (0..n.max(1))
        .map(|i| {
            let app = app.clone();
            tokio::spawn(async move { run_loop(app, i).await })
        })
        .collect()
}

async fn run_loop(app: App, worker: usize) {
    loop {
        match claim(&app).await {
            Ok(Some(job)) => {
                let job_id = job.id;
                tracing::debug!(worker, job_id, "job claimed");
                if let Err(e) = execute(&app, job).await {
                    tracing::error!(error = ?e, job_id, "job bookkeeping failed");
                }
            }
            Ok(None) => {
                tokio::select! {
                    _ = app.jobs.notified() => {}
                    _ = tokio::time::sleep(IDLE_POLL) => {}
                }
            }
            Err(e) => {
                tracing::error!(error = ?e, "claiming job failed");
                tokio::time::sleep(IDLE_POLL).await;
            }
        }
    }
}

async fn claim(app: &App) -> Result<Option<Job>> {
    let job = sqlx::query_as::<_, Job>(
        "UPDATE jobs SET state = 'running', attempts = attempts + 1
         WHERE id = (SELECT id FROM jobs WHERE state = 'queued' AND run_after <= ?
                     ORDER BY run_after, id LIMIT 1)
         RETURNING id, gif_id, attempts",
    )
    .bind(now_ms())
    .fetch_optional(&app.db)
    .await?;
    Ok(job)
}

/// True when nothing is queued or running (used by `import` to wait).
pub async fn idle(app: &App) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE state IN ('queued', 'running')")
        .fetch_one(&app.db)
        .await?;
    Ok(n == 0)
}

async fn execute(app: &App, job: Job) -> Result<()> {
    let gif_id = job.gif_id.clone().ok_or_else(|| anyhow!("job {} has no gif_id", job.id))?;
    // A panic inside the pipeline must not take the worker down.
    let result = {
        let app = app.clone();
        let gif_id = gif_id.clone();
        tokio::spawn(async move { transcode_job(&app, &gif_id).await }).await
    };
    let result = match result {
        Ok(r) => r,
        Err(join) => Err(anyhow!("worker task panicked: {join}")),
    };

    match result {
        Ok(()) => {
            sqlx::query("UPDATE jobs SET state = 'done', error = NULL WHERE id = ?")
                .bind(job.id)
                .execute(&app.db)
                .await?;
        }
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::warn!(gif_id, attempt = job.attempts, error = %msg, "transcode failed");
            let _ = tokio::fs::remove_dir_all(out_path(&app.cfg, &gif_id)).await;
            if job.attempts >= MAX_ATTEMPTS {
                sqlx::query("UPDATE jobs SET state = 'failed', error = ? WHERE id = ?")
                    .bind(&msg)
                    .bind(job.id)
                    .execute(&app.db)
                    .await?;
                sqlx::query("UPDATE gifs SET status = 'failed' WHERE id = ?")
                    .bind(&gif_id)
                    .execute(&app.db)
                    .await?;
                let _ = tokio::fs::remove_file(source_path(&app.cfg, &gif_id)).await;
            } else {
                let backoff = Duration::from_secs(5 * (1u64 << job.attempts.min(6)));
                sqlx::query("UPDATE jobs SET state = 'queued', error = ?, run_after = ? WHERE id = ?")
                    .bind(&msg)
                    .bind(now_ms() + backoff.as_millis() as i64)
                    .bind(job.id)
                    .execute(&app.db)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn transcode_job(app: &App, id: &str) -> Result<()> {
    let row = gifs::get(&app.db, id).await?.ok_or_else(|| anyhow!("gif {id} no longer exists"))?;
    if row.status == "ready" {
        return Ok(()); // finished by an earlier attempt that died before marking the job done
    }
    let kind = SourceKind::from_mime(&row.original_mime)
        .ok_or_else(|| anyhow!("unknown original mime {}", row.original_mime))?;
    let src = source_path(&app.cfg, id);
    if !src.exists() {
        return Err(anyhow!("source file {src:?} is missing"));
    }

    let out = out_path(&app.cfg, id);
    let _ = tokio::fs::remove_dir_all(&out).await;
    tokio::fs::create_dir_all(&out).await?;

    let enc = transcode::process(&app.ff, &app.cfg.media, &src, kind, &out).await?;
    finalize(app, id, kind, &src, enc).await
}

/// Move verified outputs into `media/`, apply the originals policy, mark ready.
async fn finalize(app: &App, id: &str, kind: SourceKind, src: &std::path::Path, enc: Encoded) -> Result<()> {
    let dest = media::gif_dir(&app.cfg.media_dir(), id);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let _ = tokio::fs::remove_dir_all(&dest).await; // leftovers of an earlier attempt
    tokio::fs::rename(&enc.dir, &dest).await.context("moving outputs into media/")?;

    let keep = match app.cfg.media.keep_originals {
        KeepOriginals::Always => true,
        KeepOriginals::Alpha => enc.has_alpha,
        KeepOriginals::Never => false,
    };
    let mut original: Option<(String, u64)> = None;
    if keep {
        // Copy rather than move: if we crash before the DB update, the job
        // re-runs and still finds its source.
        let name = format!("original.{}", kind.ext());
        let bytes = tokio::fs::copy(src, dest.join(&name)).await.context("keeping original")?;
        original = Some((name, bytes));
    }

    let now = now_ms();
    let mut stored = enc.full.bytes + enc.preview.bytes + enc.poster.bytes;
    if let Some((_, b)) = &original {
        stored += b;
    }
    let mut tx = app.db.begin().await?;
    sqlx::query(
        "UPDATE gifs SET status = 'ready', width = ?, height = ?, duration_ms = ?, has_alpha = ?,
                original_kept = ?, bytes_stored = ?, thumbhash = ?, phash = ? WHERE id = ?",
    )
    .bind(enc.width as i64)
    .bind(enc.height as i64)
    .bind(enc.duration_ms)
    .bind(enc.has_alpha as i64)
    .bind(original.is_some() as i64)
    .bind(stored as i64)
    .bind(&enc.thumbhash)
    .bind(enc.phash)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM variants WHERE gif_id = ?").bind(id).execute(&mut *tx).await?;
    let mut variants = vec![
        ("full.mp4".to_string(), enc.full),
        ("preview.mp4".to_string(), enc.preview),
        ("poster.webp".to_string(), enc.poster),
    ];
    if let Some((name, bytes)) = original {
        variants.push((name, transcode::FileInfo { bytes, width: enc.width, height: enc.height }));
    }
    for (name, f) in variants {
        sqlx::query(
            "INSERT INTO variants (gif_id, name, tier, bytes, width, height, created_at)
             VALUES (?, ?, 'stored', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(f.bytes as i64)
        .bind(f.width as i64)
        .bind(f.height as i64)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let _ = tokio::fs::remove_file(src).await;
    Ok(())
}
