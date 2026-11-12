//! `unigif export <file.zip>` and importing such a file back.
//!
//! The archive holds one media file per GIF (the kept original when there is
//! one, otherwise `full.mp4`) and a `manifest.json` with everything that is
//! not in the media: hashtags, description, source, dates and usage. Entries
//! are stored, not deflated: the media is already compressed.
//!
//! Trashed GIFs and GIFs that never finished processing are not exported.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::db::gifs::{self, GifRow};
use crate::db::now_ms;
use crate::ingest::{self, AddRequest};
use crate::media::{self, SourceKind};

pub const FORMAT: &str = "unigif-export";
pub const VERSION: u32 = 1;
const MANIFEST: &str = "manifest.json";

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub gifs: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// Path of the media file inside the archive.
    pub file: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub source: Source,
    pub created_at: String,
    #[serde(default)]
    pub use_count: i64,
    #[serde(default)]
    pub last_used_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Source {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Default)]
pub struct ExportSummary {
    pub gifs: usize,
    pub bytes: u64,
}

/// The file that best represents a GIF: its kept original, else `full.mp4`.
fn best_file(app: &App, row: &GifRow) -> (PathBuf, String) {
    let dir = media::gif_dir(&app.cfg.media_dir(), &row.id);
    if row.original_kept != 0 {
        if let Some(kind) = SourceKind::from_mime(&row.original_mime) {
            let name = format!("original.{}", kind.ext());
            return (dir.join(&name), kind.ext().to_string());
        }
    }
    (dir.join("full.mp4"), "mp4".to_string())
}

/// Write every ready, untrashed GIF into a zip at `dest`. The archive is
/// written next to `dest` and renamed into place once complete.
pub async fn export(app: &App, dest: &Path) -> Result<ExportSummary> {
    let rows: Vec<GifRow> = sqlx::query_as(
        "SELECT * FROM gifs WHERE deleted_at IS NULL AND status = 'ready' ORDER BY created_at, id",
    )
    .fetch_all(&app.db)
    .await?;
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut tags = gifs::tags_for(&app.db, &ids).await?;

    let mut files = Vec::with_capacity(rows.len());
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let (path, ext) = best_file(app, &row);
        let file = format!("media/{}.{ext}", row.id);
        files.push((file.clone(), path));
        entries.push(Entry {
            tags: tags.remove(&row.id).unwrap_or_default(),
            file,
            description: row.description,
            source: Source { kind: row.source, id: row.source_id, url: row.source_url },
            created_at: crate::api::types::rfc3339(row.created_at),
            use_count: row.use_count,
            last_used_at: row.last_used_at.map(crate::api::types::rfc3339),
            id: row.id,
        });
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: crate::api::types::rfc3339(now_ms()),
        gifs: entries,
    };
    let json = serde_json::to_vec_pretty(&manifest)?;

    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || write_zip(&dest, &json, &files)).await?
}

fn write_zip(dest: &Path, manifest: &[u8], files: &[(String, PathBuf)]) -> Result<ExportSummary> {
    let part = dest.with_extension("zip.part");
    let out = std::fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
    let result = (|| {
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(out));
        // Large libraries pass 4 GiB; ZIP64 is turned on per entry as needed.
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(false);
        zip.start_file(MANIFEST, opts)?;
        zip.write_all(manifest)?;
        let mut summary = ExportSummary::default();
        for (name, path) in files {
            let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
            let len = f.metadata()?.len();
            zip.start_file(name.as_str(), opts.large_file(len >= u32::MAX as u64))?;
            std::io::copy(&mut f, &mut zip).with_context(|| format!("writing {name}"))?;
            summary.gifs += 1;
            summary.bytes += len;
        }
        zip.finish()?.flush()?;
        Ok::<_, anyhow::Error>(summary)
    })();
    match result {
        Ok(s) => {
            std::fs::rename(&part, dest).with_context(|| format!("moving into {}", dest.display()))?;
            Ok(s)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub added: usize,
    pub duplicates: usize,
    pub rejected: usize,
    /// The ids of GIFs that were queued for processing.
    pub queued: Vec<String>,
}

/// Is this file an export archive (by its manifest), rather than any zip?
pub fn is_export(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .is_some_and(|mut z| z.by_name(MANIFEST).is_ok())
}

fn parse_time(s: &str) -> Option<i64> {
    let t = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()?;
    Some((t.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// Add every GIF in an export archive, restoring its hashtags, description,
/// source, dates and usage. A GIF already in the library (same bytes, or same
/// source and source id) is a duplicate: it gains the hashtags it lacks.
pub async fn import(app: &App, archive: &Path) -> Result<ImportSummary> {
    let path = archive.to_path_buf();
    let manifest: Manifest = tokio::task::spawn_blocking(move || {
        let mut z = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
        let mut text = String::new();
        z.by_name(MANIFEST).context("no manifest.json in the archive")?.read_to_string(&mut text)?;
        serde_json::from_str::<Manifest>(&text).context("reading manifest.json")
    })
    .await??;
    if manifest.format != FORMAT {
        bail!("not a unigif export (format {:?})", manifest.format);
    }
    if manifest.version > VERSION {
        bail!("export version {} is newer than this unigif understands ({VERSION})", manifest.version);
    }

    let mut summary = ImportSummary::default();
    let total = manifest.gifs.len();
    for (i, entry) in manifest.gifs.into_iter().enumerate() {
        let label = format!("[{}/{total}] {}", i + 1, entry.file);
        match import_one(app, archive, &entry).await {
            Ok((id, false)) => {
                summary.added += 1;
                summary.queued.push(id);
                println!("{label}: queued");
            }
            Ok((_, true)) => {
                summary.duplicates += 1;
                println!("{label}: duplicate");
            }
            Err(e) => {
                summary.rejected += 1;
                println!("{label}: rejected: {e:#}");
            }
        }
    }
    Ok(summary)
}

/// Returns (id, duplicate).
async fn import_one(app: &App, archive: &Path, entry: &Entry) -> Result<(String, bool)> {
    // Extract into tmp/ first: `ingest::receive` takes a stream, and the zip
    // reader is blocking.
    let tmp = app.cfg.tmp_dir().join(format!("import-{}", media::new_id()));
    let (archive_path, name, tmp_path) = (archive.to_path_buf(), entry.file.clone(), tmp.clone());
    tokio::task::spawn_blocking(move || {
        let mut z = zip::ZipArchive::new(std::fs::File::open(&archive_path)?)?;
        let mut f = z.by_name(&name).with_context(|| format!("{name} is missing from the archive"))?;
        let mut out = std::fs::File::create(&tmp_path)?;
        std::io::copy(&mut f, &mut out)?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;

    let result = async {
        let file = tokio::fs::File::open(&tmp).await?;
        let upload = ingest::receive_labeled(app, tokio_util::io::ReaderStream::new(file), "archive entry")
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut req = AddRequest::new(&entry.source.kind);
        req.tags = entry.tags.clone();
        req.source_id = entry.source.id.clone();
        req.source_url = entry.source.url.clone();
        req.description = entry.description.clone();
        let added = ingest::add(app, upload, req).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        if !added.duplicate {
            // Keep the library's history: when it was added and how often it was used.
            sqlx::query("UPDATE gifs SET created_at = ?, use_count = ?, last_used_at = ? WHERE id = ?")
                .bind(parse_time(&entry.created_at).unwrap_or(added.gif.created_at))
                .bind(entry.use_count.max(0))
                .bind(entry.last_used_at.as_deref().and_then(parse_time))
                .bind(&added.gif.id)
                .execute(&app.db)
                .await?;
        }
        Ok::<_, anyhow::Error>((added.gif.id, added.duplicate))
    }
    .await;
    let _ = tokio::fs::remove_file(&tmp).await;
    result
}
