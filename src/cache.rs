//! The on-demand conversion cache.
//!
//! `full.gif`, `preview.gif`, `full.webm` and `full.webp` are produced the
//! first time they are requested and kept under `cache_dir`. The files are
//! the truth about what is cached (deleting the whole directory is always
//! safe); the `variants` rows with `tier = 'cache'` only record size and last
//! access so the least recently used files can be evicted.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use futures_util::future::{BoxFuture, Shared};
use futures_util::FutureExt;

use crate::app::App;
use crate::db::gifs::GifRow;
use crate::db::{now_ms, Db};
use crate::media::convert::{self, Source};
use crate::media::{SourceKind, Variant};

type Flight = Shared<BoxFuture<'static, Result<(), String>>>;

/// Conversions in progress, keyed by `id/name`, so concurrent requests for
/// the same file share one ffmpeg run.
#[derive(Default)]
pub struct InFlight(Mutex<HashMap<String, Flight>>);

/// Make sure `variant` of `row` exists in the cache and return its path.
pub async fn ensure(app: &App, row: &GifRow, variant: Variant) -> Result<PathBuf> {
    let path = app.cfg.cache_gif_dir(&row.id).join(variant.file_name());
    if let Ok(meta) = tokio::fs::metadata(&path).await {
        if meta.len() > 0 {
            touch(&app.db, &row.id, variant, meta.len()).await?;
            return Ok(path);
        }
    }

    let key = format!("{}/{}", row.id, variant.file_name());
    let flight = {
        let mut map = app.inflight.0.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key.clone())
            .or_insert_with(|| {
                // Detached from the request: if the client goes away the
                // conversion still finishes and the next request is a hit.
                let app = app.clone();
                let row = row.clone();
                let task_key = key.clone();
                let handle = tokio::spawn(async move {
                    let r = generate(&app, &row, variant).await;
                    app.inflight.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&task_key);
                    r
                });
                async move {
                    match handle.await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(format!("{e:#}")),
                        Err(join) => Err(format!("conversion task panicked: {join}")),
                    }
                }
                .boxed()
                .shared()
            })
            .clone()
    };
    flight.await.map_err(|e| anyhow!(e))?;
    Ok(path)
}

/// Which file a conversion reads. The kept original is used for the image
/// outputs when it is an image (best quality, and the only way transparency
/// survives); everything else is made from `full.mp4`.
fn pick_source(app: &App, row: &GifRow, variant: Variant) -> (PathBuf, Option<&'static str>, bool) {
    let dir = crate::media::gif_dir(&app.cfg.media_dir(), &row.id);
    let image_out = matches!(variant, Variant::FullGif | Variant::PreviewGif | Variant::FullWebp);
    if image_out && row.original_kept != 0 {
        if let Some(kind) = SourceKind::from_mime(&row.original_mime) {
            if matches!(kind, SourceKind::Gif | SourceKind::Webp | SourceKind::Png) {
                return (dir.join(format!("original.{}", kind.ext())), kind.demuxer(), true);
            }
        }
    }
    (dir.join("full.mp4"), Some("mov"), false)
}

async fn generate(app: &App, row: &GifRow, variant: Variant) -> Result<()> {
    let dir = app.cfg.cache_gif_dir(&row.id);
    tokio::fs::create_dir_all(&dir).await.context("creating cache directory")?;
    let final_path = dir.join(variant.file_name());
    let part = dir.join(format!("{}.part", variant.file_name()));
    let _ = tokio::fs::remove_file(&part).await;

    let (src_path, demuxer, is_original) = pick_source(app, row, variant);
    let src = Source { path: &src_path, demuxer, is_original };
    let result = async {
        convert::convert(&app.ff, &app.cfg.media, variant, src, row.has_alpha != 0, &part).await?;
        tokio::fs::rename(&part, &final_path).await.context("moving the conversion into the cache")?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }

    let len = tokio::fs::metadata(&final_path).await?.len();
    touch(&app.db, &row.id, variant, len).await?;
    if let Err(e) = evict(app, Some((&row.id, variant.file_name()))).await {
        tracing::warn!(error = ?e, "cache eviction failed");
    }
    Ok(())
}

/// Record a use of a cached file (inserting its row if needed).
async fn touch(db: &Db, id: &str, variant: Variant, bytes: u64) -> Result<(), sqlx::Error> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO variants (gif_id, name, tier, bytes, created_at, last_access_at)
         VALUES (?, ?, 'cache', ?, ?, ?)
         ON CONFLICT (gif_id, name) DO UPDATE SET bytes = excluded.bytes, last_access_at = excluded.last_access_at",
    )
    .bind(id)
    .bind(variant.file_name())
    .bind(bytes as i64)
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

/// Delete least recently used files until the cache fits `cache.maxSize`.
/// `keep` is never removed (the file a request is about to serve). Returns the
/// number of files and bytes evicted.
pub async fn evict(app: &App, keep: Option<(&str, &str)>) -> Result<(usize, u64)> {
    let max = app.cfg.cache.max_size.0 as i64;
    let recorded: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(bytes), 0) FROM variants WHERE tier = 'cache'")
            .fetch_one(&app.db)
            .await?;
    if recorded <= max {
        return Ok((0, 0));
    }
    // One eviction at a time, so two runs cannot both free the same bytes.
    let _guard = app.evicting.lock().await;

    // The directory may have been emptied or edited by hand, so before
    // deleting anything check the rows against what is really on disk.
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT gif_id, name, bytes FROM variants WHERE tier = 'cache'
         ORDER BY COALESCE(last_access_at, created_at), gif_id, name",
    )
    .fetch_all(&app.db)
    .await?;
    let mut live: Vec<(String, String, i64)> = Vec::with_capacity(rows.len());
    for (id, name, bytes) in rows {
        let path = app.cfg.cache_gif_dir(&id).join(&name);
        match tokio::fs::metadata(&path).await {
            Ok(m) => {
                let actual = m.len() as i64;
                if actual != bytes {
                    sqlx::query("UPDATE variants SET bytes = ? WHERE gif_id = ? AND name = ?")
                        .bind(actual)
                        .bind(&id)
                        .bind(&name)
                        .execute(&app.db)
                        .await?;
                }
                live.push((id, name, actual));
            }
            Err(_) => {
                sqlx::query("DELETE FROM variants WHERE gif_id = ? AND name = ? AND tier = 'cache'")
                    .bind(&id)
                    .bind(&name)
                    .execute(&app.db)
                    .await?;
            }
        }
    }

    let mut total: i64 = live.iter().map(|(_, _, b)| b).sum();
    let (mut evicted, mut freed) = (0, 0u64);
    for (id, name, bytes) in live {
        if total <= max {
            break;
        }
        if keep.is_some_and(|(kid, kname)| kid == id && kname == name) {
            continue;
        }
        let dir = app.cfg.cache_gif_dir(&id);
        let _ = tokio::fs::remove_file(dir.join(&name)).await;
        let _ = tokio::fs::remove_dir(&dir).await; // only succeeds once empty
        sqlx::query("DELETE FROM variants WHERE gif_id = ? AND name = ? AND tier = 'cache'")
            .bind(&id)
            .bind(&name)
            .execute(&app.db)
            .await?;
        total -= bytes;
        evicted += 1;
        freed += bytes as u64;
    }
    Ok((evicted, freed))
}

/// Empty the cache now (`DELETE /cache`, `unigif gc` does not). Conversions
/// in progress are skipped: their `.part` files and the finished files a
/// request is waiting on. A file that is being streamed keeps streaming (the
/// open handle outlives the unlink). Returns (files, bytes) removed.
pub async fn clear(app: &App) -> Result<(u64, u64)> {
    let _guard = app.evicting.lock().await;
    let busy: std::collections::HashSet<String> =
        app.inflight.0.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect();
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut shards = match tokio::fs::read_dir(&app.cfg.cache_dir).await {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(e) => return Err(e).context("reading the cache directory"),
    };
    while let Some(shard) = shards.next_entry().await? {
        if !shard.file_type().await?.is_dir() {
            continue;
        }
        let mut gifs = tokio::fs::read_dir(shard.path()).await?;
        while let Some(gif) = gifs.next_entry().await? {
            let id = gif.file_name().to_string_lossy().into_owned();
            if !gif.file_type().await?.is_dir() || !crate::media::valid_id(&id) {
                continue;
            }
            let mut entries = tokio::fs::read_dir(gif.path()).await?;
            while let Some(f) = entries.next_entry().await? {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.ends_with(".part") || busy.contains(&format!("{id}/{name}")) {
                    continue;
                }
                let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
                if tokio::fs::remove_file(f.path()).await.is_ok() {
                    files += 1;
                    bytes += len;
                    sqlx::query("DELETE FROM variants WHERE gif_id = ? AND name = ? AND tier = 'cache'")
                        .bind(&id)
                        .bind(&name)
                        .execute(&app.db)
                        .await?;
                }
            }
            let _ = tokio::fs::remove_dir(gif.path()).await; // only succeeds once empty
        }
        let _ = tokio::fs::remove_dir(shard.path()).await;
    }
    // Rows whose files were already gone (the directory edited by hand).
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT gif_id, name FROM variants WHERE tier = 'cache'").fetch_all(&app.db).await?;
    for (id, name) in rows {
        if tokio::fs::metadata(app.cfg.cache_gif_dir(&id).join(&name)).await.is_err() {
            sqlx::query("DELETE FROM variants WHERE gif_id = ? AND name = ? AND tier = 'cache'")
                .bind(&id)
                .bind(&name)
                .execute(&app.db)
                .await?;
        }
    }
    Ok((files, bytes))
}

/// Files and bytes in the cache, for the storage numbers.
pub async fn totals(db: &Db, vis: &crate::db::gifs::Visibility) -> Result<(i64, i64), sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::new(
        "SELECT COUNT(*), COALESCE(SUM(v.bytes), 0) FROM variants v JOIN gifs g ON g.id = v.gif_id
         WHERE v.tier = 'cache'",
    );
    vis.push_clause(&mut qb, "g");
    qb.build_query_as().fetch_one(db).await
}
