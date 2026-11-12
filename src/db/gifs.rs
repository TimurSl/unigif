//! Row types and plain lookups for `gifs`, `variants` and tag names.

use sqlx::FromRow;

use super::Db;

#[derive(Debug, Clone, FromRow)]
pub struct GifRow {
    pub id: String,
    pub status: String,
    pub sha256: Vec<u8>,
    pub phash: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub has_alpha: i64,
    pub original_mime: String,
    pub original_kept: i64,
    pub bytes_original: i64,
    pub bytes_stored: i64,
    pub thumbhash: Option<Vec<u8>>,
    pub description: Option<String>,
    pub source: String,
    pub source_id: Option<String>,
    pub source_url: Option<String>,
    pub added_by_key: Option<String>,
    pub tag_count: i64,
    pub use_count: i64,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub sorted_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct VariantRow {
    pub name: String,
    pub tier: String,
    pub bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

pub async fn get(db: &Db, id: &str) -> Result<Option<GifRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM gifs WHERE id = ?").bind(id).fetch_optional(db).await
}

pub async fn find_by_sha(db: &Db, sha: &[u8]) -> Result<Option<GifRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM gifs WHERE sha256 = ?").bind(sha).fetch_optional(db).await
}

pub async fn find_by_source(db: &Db, source: &str, source_id: &str) -> Result<Option<GifRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM gifs WHERE source = ? AND source_id = ?")
        .bind(source)
        .bind(source_id)
        .fetch_optional(db)
        .await
}

pub async fn tags_of(db: &Db, id: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT t.name FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id
         WHERE gt.gif_id = ? ORDER BY gt.created_at, t.name",
    )
    .bind(id)
    .fetch_all(db)
    .await
}

pub async fn variants_of(db: &Db, id: &str) -> Result<Vec<VariantRow>, sqlx::Error> {
    sqlx::query_as("SELECT name, tier, bytes, width, height FROM variants WHERE gif_id = ? ORDER BY name")
        .bind(id)
        .fetch_all(db)
        .await
}
