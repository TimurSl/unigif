//! Ingest: receive bytes → deduplicate → sniff → probe → insert → queue.
//! The heavy work (transcoding) happens in [`worker`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::app::App;
use crate::db::gifs::{self, GifRow};
use crate::db::now_ms;
use crate::error::ApiError;
use crate::media::{self, probe, SourceKind};
use crate::tags;

pub mod worker;

pub const SOURCES: [&str; 6] = ["web", "api", "url", "share", "telegram", "import"];

/// A file received into `tmp/`. Deleted on drop; a rename away makes the
/// drop a no-op.
pub struct Upload {
    pub path: PathBuf,
    pub sha256: [u8; 32],
    pub size: u64,
}

impl Drop for Upload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub tags: Vec<String>,
    pub source: String,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub description: Option<String>,
    pub added_by_key: Option<String>,
    pub wait: Duration,
}

impl AddRequest {
    pub fn new(source: &str) -> Self {
        Self {
            tags: vec![],
            source: source.to_string(),
            source_id: None,
            source_url: None,
            description: None,
            added_by_key: None,
            wait: Duration::ZERO,
        }
    }
}

#[derive(Debug)]
pub struct Added {
    pub gif: GifRow,
    pub duplicate: bool,
}

/// Stream a request body into `tmp/`, hashing it and enforcing the size cap.
pub async fn receive<S, E>(app: &App, body: S) -> Result<Upload, ApiError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    receive_labeled(app, body, "request body").await
}

/// [`receive`], naming what is being read in error messages.
pub async fn receive_labeled<S, E>(app: &App, mut body: S, what: &str) -> Result<Upload, ApiError>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    let limit = app.cfg.upload.max_size.0;
    let dir = app.cfg.tmp_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(anyhow::Error::from)?;
    let path = dir.join(format!("upload-{}", media::new_id()));
    // Guard first, so every early return below removes the partial file.
    let mut up = Upload { path: path.clone(), sha256: [0; 32], size: 0 };
    let mut file = tokio::fs::File::create(&path).await.map_err(anyhow::Error::from)?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| ApiError::BadRequest(format!("reading {what}: {e}")))?;
        size += chunk.len() as u64;
        if size > limit {
            return Err(ApiError::TooLarge { limit });
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
    }
    file.flush().await.map_err(anyhow::Error::from)?;
    if size == 0 {
        return Err(ApiError::BadRequest("empty upload".into()));
    }
    up.sha256 = hasher.finalize().into();
    up.size = size;
    Ok(up)
}

/// Add an uploaded file to the library.
pub async fn add(app: &App, upload: Upload, mut req: AddRequest) -> Result<Added, ApiError> {
    req.tags = tags::normalize_all(&req.tags).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if !SOURCES.contains(&req.source.as_str()) {
        return Err(ApiError::BadRequest(format!("source must be one of {}", SOURCES.join(", "))));
    }
    if req.source_id.as_deref().is_some_and(|s| s.is_empty() || s.len() > 256) {
        return Err(ApiError::BadRequest("source_id must be 1 to 256 characters".into()));
    }
    if req.description.as_deref().is_some_and(|d| d.len() > 2000) {
        return Err(ApiError::BadRequest("description is longer than 2000 bytes".into()));
    }

    // 1. Deduplicate by content or by (source, source_id).
    if let Some(existing) = find_existing(app, &upload, &req).await? {
        return merge_duplicate(app, existing, &req).await;
    }

    // 2. Sniff from content.
    let head = read_head(&upload.path).await?;
    let kind = SourceKind::sniff(&head).ok_or_else(|| {
        ApiError::UnsupportedMedia("unsupported file type; send a GIF, WebP, PNG, MP4, WebM or MKV".into())
    })?;

    // 3. Probe and enforce limits before anything decodes the file.
    let info = probe::probe_source(&app.ff, &upload.path, kind).await.map_err(|e| {
        tracing::debug!(error = ?e, "probe failed");
        ApiError::InvalidMedia("the file is not a decodable animation or video".into())
    })?;
    probe::check_limits(&info, &app.cfg.media).map_err(ApiError::InvalidMedia)?;
    probe::check_decodes(&app.ff, &upload.path, kind).await.map_err(|e| {
        tracing::debug!(error = ?e, "first-frame decode failed");
        ApiError::InvalidMedia("the file is not a decodable animation or video".into())
    })?;

    // 4. Park the source where it survives a restart, insert, queue.
    let id = media::new_id();
    tokio::fs::create_dir_all(app.cfg.work_dir()).await.map_err(anyhow::Error::from)?;
    let work = worker::source_path(&app.cfg, &id);
    move_file(&upload.path, &work).await.map_err(anyhow::Error::from)?;

    match insert(app, &id, &upload, kind, &req).await {
        Ok(()) => {}
        Err(e) if is_unique_violation(&e) => {
            // Lost a race with a concurrent identical upload.
            let _ = tokio::fs::remove_file(&work).await;
            if let Some(existing) = find_existing(app, &upload, &req).await? {
                return merge_duplicate(app, existing, &req).await;
            }
            return Err(e.into());
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&work).await;
            return Err(e.into());
        }
    }
    app.jobs.notify_one();
    app.emit("gif.added", &id);

    // 5. Optionally wait for the worker so most callers get a finished item.
    let deadline = Instant::now() + req.wait;
    let mut row = gifs::get(&app.db, &id).await?.ok_or_else(|| anyhow::anyhow!("row vanished"))?;
    while row.status == "processing" && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        row = gifs::get(&app.db, &id).await?.ok_or_else(|| anyhow::anyhow!("row vanished"))?;
    }
    Ok(Added { gif: row, duplicate: false })
}

async fn find_existing(app: &App, up: &Upload, req: &AddRequest) -> Result<Option<GifRow>, ApiError> {
    let mut found = gifs::find_by_sha(&app.db, &up.sha256).await?;
    if found.is_none() {
        if let Some(sid) = &req.source_id {
            found = gifs::find_by_source(&app.db, &req.source, sid).await?;
        }
    }
    // A failed item is dead weight that would block a retry forever: drop it
    // and let the new upload take its place.
    if let Some(row) = &found {
        if row.status == "failed" {
            sqlx::query("DELETE FROM gifs WHERE id = ?").bind(&row.id).execute(&app.db).await?;
            let _ = tokio::fs::remove_dir_all(media::gif_dir(&app.cfg.media_dir(), &row.id)).await;
            return Ok(None);
        }
    }
    Ok(found)
}

async fn merge_duplicate(app: &App, existing: GifRow, req: &AddRequest) -> Result<Added, ApiError> {
    let mut tx = crate::db::begin_write(&app.db).await?;
    let now = now_ms();
    // Uploading a trashed GIF again brings it back (restoring also puts its
    // hashtags back into the tag counts).
    let restored = existing.deleted_at.is_some() && gifs::restore(&mut tx, &existing.id).await?;
    let added_tags = tags::add_tags(&mut tx, &existing.id, &req.tags, now).await.map_err(|e| match e {
        tags::AddTagsError::Tag(t) => ApiError::BadRequest(t.to_string()),
        tags::AddTagsError::Db(d) => d.into(),
    })?;
    tx.commit().await?;
    if restored {
        app.emit("gif.added", &existing.id);
    }
    if added_tags > 0 {
        app.emit("gif.tagged", &existing.id);
    }
    let gif = gifs::get(&app.db, &existing.id).await?.ok_or_else(|| anyhow::anyhow!("row vanished"))?;
    Ok(Added { gif, duplicate: true })
}

async fn insert(
    app: &App,
    id: &str,
    up: &Upload,
    kind: SourceKind,
    req: &AddRequest,
) -> Result<(), sqlx::Error> {
    let now = now_ms();
    let mut tx = crate::db::begin_write(&app.db).await?;
    sqlx::query(
        "INSERT INTO gifs (id, status, sha256, original_mime, bytes_original, description,
                           source, source_id, source_url, added_by_key, created_at)
         VALUES (?, 'processing', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&up.sha256[..])
    .bind(kind.mime())
    .bind(up.size as i64)
    .bind(&req.description)
    .bind(&req.source)
    .bind(&req.source_id)
    .bind(&req.source_url)
    .bind(&req.added_by_key)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if let Err(e) = tags::add_tags(&mut tx, id, &req.tags, now).await {
        return Err(match e {
            tags::AddTagsError::Db(d) => d,
            // normalize_all already enforced the cap; unreachable in practice.
            tags::AddTagsError::Tag(t) => sqlx::Error::Protocol(t.to_string()),
        });
    }
    sqlx::query("INSERT INTO jobs (kind, gif_id, state, run_after, created_at) VALUES ('transcode', ?, 'queued', ?, ?)")
        .bind(id)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

async fn read_head(path: &Path) -> Result<Vec<u8>, ApiError> {
    use tokio::io::AsyncReadExt;
    let mut f = tokio::fs::File::open(path).await.map_err(anyhow::Error::from)?;
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf).await.map_err(anyhow::Error::from)?;
    buf.truncate(n);
    Ok(buf)
}

/// Rename, falling back to copy + delete across filesystems.
pub async fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if tokio::fs::rename(from, to).await.is_ok() {
        return Ok(());
    }
    tokio::fs::copy(from, to).await?;
    tokio::fs::remove_file(from).await
}
