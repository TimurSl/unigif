//! JSON shapes of the public API (section 8.2 of the plan).

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::Config;
use crate::db::gifs::{GifRow, VariantRow};

#[derive(Debug, Serialize)]
pub struct GifObject {
    pub id: String,
    pub status: String,
    pub tags: Vec<String>,
    pub description: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub has_alpha: bool,
    /// Size of the file as it was added.
    pub bytes_original: i64,
    /// Whether that file is kept next to the MP4 (GIF, WebP and PNG sources
    /// are, for the best image conversions and for transparency).
    pub original_kept: bool,
    pub thumbhash: Option<String>,
    pub source: SourceObject,
    pub created_at: String,
    pub sorted_at: Option<String>,
    /// Public share page with OpenGraph tags.
    pub page_url: String,
    pub media: Option<MediaObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SourceObject {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MediaObject {
    pub mp4: Option<MediaFile>,
    pub preview_mp4: Option<MediaFile>,
    pub poster: Option<MediaFile>,
    pub gif: OnDemandFile,
    pub preview_gif: OnDemandFile,
    pub webm: OnDemandFile,
    pub webp: OnDemandFile,
}

/// A variant that is converted the first time its URL is fetched.
#[derive(Debug, Serialize)]
pub struct OnDemandFile {
    pub url: String,
    pub on_demand: bool,
}

#[derive(Debug, Serialize)]
pub struct MediaFile {
    pub url: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: i64,
}

pub fn rfc3339(ms: i64) -> String {
    OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

pub fn page_url(cfg: &Config, id: &str) -> String {
    format!("{}/g/{id}", cfg.base_url())
}

pub fn media_url(cfg: &Config, id: &str, variant: &str) -> String {
    format!("{}/m/{id}/{variant}", cfg.base_url())
}

impl GifObject {
    pub fn build(cfg: &Config, row: GifRow, tags: Vec<String>, variants: &[VariantRow]) -> Self {
        let file = |name: &str| {
            variants.iter().find(|v| v.name == name).map(|v| MediaFile {
                url: media_url(cfg, &row.id, name),
                width: v.width,
                height: v.height,
                bytes: v.bytes,
            })
        };
        let lazy = |name: &str| OnDemandFile { url: media_url(cfg, &row.id, name), on_demand: true };
        let media = (row.status == "ready").then(|| MediaObject {
            mp4: file("full.mp4"),
            preview_mp4: file("preview.mp4"),
            poster: file("poster.webp"),
            gif: lazy("full.gif"),
            preview_gif: lazy("preview.gif"),
            webm: lazy("full.webm"),
            webp: lazy("full.webp"),
        });
        let page = page_url(cfg, &row.id);
        Self {
            thumbhash: row.thumbhash.as_deref().map(|t| STANDARD_NO_PAD.encode(t)),
            source: SourceObject { kind: row.source, id: row.source_id },
            created_at: rfc3339(row.created_at),
            sorted_at: row.sorted_at.map(rfc3339),
            has_alpha: row.has_alpha != 0,
            bytes_original: row.bytes_original,
            original_kept: row.original_kept != 0,
            id: row.id,
            status: row.status,
            tags,
            description: row.description,
            width: row.width,
            height: row.height,
            duration_ms: row.duration_ms,
            page_url: page,
            media,
            duplicate: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_783_848_600_000), "2026-07-12T09:30:00Z");
    }
}

impl GifObject {
    /// Show only the hashtags in `allowed` and no media: what a credential
    /// that may not look at this GIF gets back when its upload turns out to be
    /// a duplicate of something it cannot see.
    pub fn redact(&mut self, allowed: &[String]) {
        self.tags.retain(|t| allowed.contains(t));
        self.media = None;
    }
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TagObject {
    pub name: String,
    pub gif_count: i64,
    pub last_used_at: Option<String>,
}
