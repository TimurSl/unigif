//! Row types and plain lookups for `gifs`, `variants` and tag names.

use std::collections::HashMap;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sqlx::{FromRow, QueryBuilder, Sqlite, Transaction};

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

// ---------------------------------------------------------------------------
// Visibility: per-key hidden hashtags, applied in the query layer.
// ---------------------------------------------------------------------------

/// Which GIFs a credential may see: everything except GIFs carrying one of
/// the `deny` hashtags. Every query that returns or touches GIFs on behalf of
/// a caller takes one, so by-ID lookups cannot bypass it.
#[derive(Debug, Clone, Default)]
pub struct Visibility {
    pub deny: Vec<String>,
}

impl Visibility {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn is_all(&self) -> bool {
        self.deny.is_empty()
    }

    /// Append ` AND {alias}.id NOT IN (…)` for the GIF aliased `alias`. The
    /// subquery is not correlated, so SQLite builds the set of hidden GIFs once
    /// per statement instead of probing per row (a correlated `NOT EXISTS`
    /// made full scans such as `/stats` take most of a second at 50 000 GIFs).
    pub fn push_clause(&self, qb: &mut QueryBuilder<'_, Sqlite>, alias: &str) {
        if self.deny.is_empty() {
            return;
        }
        qb.push(format!(
            " AND {alias}.id NOT IN (SELECT dgt.gif_id FROM gif_tags dgt
               WHERE dgt.tag_id IN (SELECT dt.id FROM tags dt WHERE dt.name IN ("
        ));
        let mut sep = qb.separated(", ");
        for d in &self.deny {
            sep.push_bind(d.clone());
        }
        qb.push(")))");
    }

    /// Push a subquery `(tag_id, n)`: for each hashtag, how many GIFs outside
    /// the trash carry it and are hidden from the credential. Subtracted from
    /// the maintained `tags.gif_count`, this gives visible counts at a cost
    /// that grows with the hidden GIFs rather than with the library.
    pub fn push_hidden_tag_counts(&self, qb: &mut QueryBuilder<'_, Sqlite>) {
        qb.push(
            "(SELECT hgt.tag_id AS tag_id, COUNT(*) AS n FROM gif_tags hgt JOIN gifs hg ON hg.id = hgt.gif_id
              WHERE hg.deleted_at IS NULL AND hgt.gif_id IN (SELECT dgt.gif_id FROM gif_tags dgt
                WHERE dgt.tag_id IN (SELECT dt.id FROM tags dt WHERE dt.name IN (",
        );
        let mut sep = qb.separated(", ");
        for d in &self.deny {
            sep.push_bind(d.clone());
        }
        qb.push("))) GROUP BY hgt.tag_id)");
    }

    /// Is this GIF hidden from the credential (ignoring trash)?
    pub async fn hides(&self, db: &Db, gif_id: &str) -> Result<bool, sqlx::Error> {
        if self.deny.is_empty() {
            return Ok(false);
        }
        let mut qb = QueryBuilder::new("SELECT EXISTS (SELECT 1 FROM gifs g WHERE g.id = ");
        qb.push_bind(gif_id.to_string());
        qb.push(" AND NOT (1=1");
        self.push_clause(&mut qb, "g");
        qb.push("))");
        qb.build_query_scalar::<i64>().fetch_one(db).await.map(|v| v != 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trash {
    /// Only GIFs that are not in the trash.
    Without,
    /// Only trashed GIFs.
    Only,
    Any,
}

/// One GIF as seen by a credential. `None` when it does not exist, is hidden,
/// or is in the wrong trash state.
pub async fn get_visible(
    db: &Db,
    id: &str,
    vis: &Visibility,
    trash: Trash,
) -> Result<Option<GifRow>, sqlx::Error> {
    let mut qb = QueryBuilder::new("SELECT g.* FROM gifs g WHERE g.id = ");
    qb.push_bind(id.to_string());
    match trash {
        Trash::Without => qb.push(" AND g.deleted_at IS NULL"),
        Trash::Only => qb.push(" AND g.deleted_at IS NOT NULL"),
        Trash::Any => &mut qb,
    };
    vis.push_clause(&mut qb, "g");
    qb.build_query_as::<GifRow>().fetch_optional(db).await
}

// ---------------------------------------------------------------------------
// Listing and search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    New,
    Old,
    Used,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "new" => Some(Self::New),
            "old" => Some(Self::Old),
            "used" => Some(Self::Used),
            _ => None,
        }
    }
}

/// Keyset position. `n1` is `created_at` for `new`/`old` and `use_count` for
/// `used`; `n2` is `created_at` for `used`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub n1: i64,
    pub n2: i64,
    pub id: String,
}

impl Cursor {
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{}|{}|{}", self.n1, self.n2, self.id))
    }

    pub fn decode(s: &str) -> Option<Self> {
        let raw = String::from_utf8(URL_SAFE_NO_PAD.decode(s).ok()?).ok()?;
        let mut it = raw.splitn(3, '|');
        let n1 = it.next()?.parse().ok()?;
        let n2 = it.next()?.parse().ok()?;
        let id = it.next()?.to_string();
        crate::media::valid_id(&id).then_some(Self { n1, n2, id })
    }

    fn of(row: &GifRow, sort: Sort) -> Self {
        match sort {
            Sort::New | Sort::Old => Self { n1: row.created_at, n2: 0, id: row.id.clone() },
            Sort::Used => Self { n1: row.use_count, n2: row.created_at, id: row.id.clone() },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListParams {
    /// Hashtags a GIF must carry (exact, already normalised).
    pub tags: Vec<String>,
    /// A GIF must carry a hashtag starting with this.
    pub prefix: Option<String>,
    /// Set by [`list`]: the prefix covers many GIFs, so walking the library
    /// in order and stopping at `limit` beats collecting every match first.
    pub prefix_broad: bool,
    /// Hashtags a GIF must not carry.
    pub exclude: Vec<String>,
    pub unsorted: bool,
    /// Matches nothing (a search token that cannot be a hashtag).
    pub impossible: bool,
    pub statuses: Vec<String>,
    pub trash: bool,
    pub sort: Sort,
    pub cursor: Option<Cursor>,
    pub limit: i64,
}

fn push_filters(qb: &mut QueryBuilder<'_, Sqlite>, p: &ListParams, vis: &Visibility) {
    qb.push(if p.trash { " WHERE g.deleted_at IS NOT NULL" } else { " WHERE g.deleted_at IS NULL" });
    if p.impossible {
        qb.push(" AND 0");
    }
    if !p.statuses.is_empty() {
        qb.push(" AND g.status IN (");
        let mut sep = qb.separated(", ");
        for s in &p.statuses {
            sep.push_bind(s.clone());
        }
        qb.push(")");
    }
    if p.unsorted {
        qb.push(" AND g.tag_count = 0");
    }
    for t in &p.tags {
        qb.push(" AND g.id IN (SELECT gt.gif_id FROM gif_tags gt WHERE gt.tag_id = (SELECT id FROM tags WHERE name = ");
        qb.push_bind(t.clone());
        qb.push("))");
    }
    if let Some(prefix) = &p.prefix {
        // A range scan on the unique index on tags.name; no LIKE escaping needed.
        if p.prefix_broad {
            qb.push(" AND EXISTS (SELECT 1 FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id WHERE gt.gif_id = g.id AND t.name >= ");
        } else {
            qb.push(" AND g.id IN (SELECT gt.gif_id FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id WHERE t.name >= ");
        }
        qb.push_bind(prefix.clone());
        qb.push(" AND t.name <= ");
        qb.push_bind(format!("{prefix}\u{10FFFF}"));
        qb.push(")");
    }
    for t in &p.exclude {
        qb.push(" AND g.id NOT IN (SELECT gt.gif_id FROM gif_tags gt WHERE gt.tag_id = (SELECT id FROM tags WHERE name = ");
        qb.push_bind(t.clone());
        qb.push("))");
    }
    vis.push_clause(qb, "g");
}

fn push_cursor(qb: &mut QueryBuilder<'_, Sqlite>, p: &ListParams, c: &Cursor) {
    match p.sort {
        Sort::New => {
            qb.push(" AND (g.created_at, g.id) < (");
            qb.push_bind(c.n1).push(", ").push_bind(c.id.clone()).push(")");
        }
        Sort::Old => {
            qb.push(" AND (g.created_at, g.id) > (");
            qb.push_bind(c.n1).push(", ").push_bind(c.id.clone()).push(")");
        }
        Sort::Used => {
            qb.push(" AND (g.use_count, g.created_at, g.id) < (");
            qb.push_bind(c.n1).push(", ").push_bind(c.n2).push(", ").push_bind(c.id.clone()).push(")");
        }
    }
}

/// Above this many tagged GIFs a prefix counts as broad (see `prefix_broad`).
/// Measured at 50 000 GIFs: collecting the matches costs about one row each,
/// walking the library costs about `limit × library / matches`.
const BROAD_PREFIX: i64 = 1000;

/// How many GIFs carry a hashtag starting with `prefix`, from the maintained
/// counts (an upper bound: a GIF with two matching hashtags counts twice).
async fn prefix_coverage(db: &Db, prefix: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(SUM(gif_count), 0) FROM tags WHERE name >= ? AND name <= ?")
        .bind(prefix)
        .bind(format!("{prefix}\u{10FFFF}"))
        .fetch_one(db)
        .await
}

/// A page of GIFs and the cursor for the next one.
pub async fn list(
    db: &Db,
    p: &ListParams,
    vis: &Visibility,
) -> Result<(Vec<GifRow>, Option<String>), sqlx::Error> {
    let mut broad = None;
    if let Some(prefix) = &p.prefix {
        if !p.trash && prefix_coverage(db, prefix).await? > BROAD_PREFIX {
            broad = Some(ListParams { prefix_broad: true, ..p.clone() });
        }
    }
    let p = broad.as_ref().unwrap_or(p);
    let mut qb = QueryBuilder::new("SELECT g.* FROM gifs g");
    push_filters(&mut qb, p, vis);
    if let Some(c) = &p.cursor {
        push_cursor(&mut qb, p, c);
    }
    qb.push(match p.sort {
        Sort::New => " ORDER BY g.created_at DESC, g.id DESC",
        Sort::Old => " ORDER BY g.created_at ASC, g.id ASC",
        Sort::Used => " ORDER BY g.use_count DESC, g.created_at DESC, g.id DESC",
    });
    qb.push(" LIMIT ").push_bind(p.limit + 1);
    let mut rows: Vec<GifRow> = qb.build_query_as().fetch_all(db).await?;
    let next = if rows.len() as i64 > p.limit {
        rows.truncate(p.limit as usize);
        rows.last().map(|r| Cursor::of(r, p.sort).encode())
    } else {
        None
    };
    Ok((rows, next))
}

/// How many GIFs match (ignores `limit`; honours the cursor).
pub async fn count(db: &Db, p: &ListParams, vis: &Visibility) -> Result<i64, sqlx::Error> {
    let mut qb = QueryBuilder::new("SELECT COUNT(*) FROM gifs g");
    push_filters(&mut qb, p, vis);
    if let Some(c) = &p.cursor {
        push_cursor(&mut qb, p, c);
    }
    qb.build_query_scalar().fetch_one(db).await
}

/// Cursor that positions a list just after an existing GIF.
pub fn cursor_after(row: &GifRow, sort: Sort) -> Cursor {
    Cursor::of(row, sort)
}

// ---------------------------------------------------------------------------
// Batch loads
// ---------------------------------------------------------------------------

pub async fn tags_for(db: &Db, ids: &[String]) -> Result<HashMap<String, Vec<String>>, sqlx::Error> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut qb = QueryBuilder::new(
        "SELECT gt.gif_id, t.name FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id WHERE gt.gif_id IN (",
    );
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(id.clone());
    }
    qb.push(") ORDER BY gt.created_at, t.name");
    let rows: Vec<(String, String)> = qb.build_query_as().fetch_all(db).await?;
    for (gif, tag) in rows {
        out.entry(gif).or_default().push(tag);
    }
    Ok(out)
}

pub async fn variants_for(db: &Db, ids: &[String]) -> Result<HashMap<String, Vec<VariantRow>>, sqlx::Error> {
    #[derive(FromRow)]
    struct Row {
        gif_id: String,
        name: String,
        tier: String,
        bytes: i64,
        width: Option<i64>,
        height: Option<i64>,
    }
    let mut out: HashMap<String, Vec<VariantRow>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut qb =
        QueryBuilder::new("SELECT gif_id, name, tier, bytes, width, height FROM variants WHERE gif_id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(id.clone());
    }
    qb.push(") ORDER BY name");
    let rows: Vec<Row> = qb.build_query_as().fetch_all(db).await?;
    for r in rows {
        out.entry(r.gif_id).or_default().push(VariantRow {
            name: r.name,
            tier: r.tier,
            bytes: r.bytes,
            width: r.width,
            height: r.height,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Trash
// ---------------------------------------------------------------------------

/// Move a GIF to the trash. Its hashtags stay attached but stop counting
/// towards `tags.gif_count`. Returns whether anything changed.
pub async fn trash(tx: &mut Transaction<'_, Sqlite>, id: &str, now: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE gifs SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("UPDATE tags SET gif_count = gif_count - 1 WHERE id IN (SELECT tag_id FROM gif_tags WHERE gif_id = ?)")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(true)
}

/// Take a GIF out of the trash. Returns whether anything changed.
pub async fn restore(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE gifs SET deleted_at = NULL WHERE id = ? AND deleted_at IS NOT NULL")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    if r.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("UPDATE tags SET gif_count = gif_count + 1 WHERE id IN (SELECT tag_id FROM gif_tags WHERE gif_id = ?)")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, FromRow)]
pub struct Totals {
    pub gifs: i64,
    pub unsorted: i64,
    pub bytes_stored: i64,
    pub bytes_original: i64,
}

pub async fn totals(db: &Db, vis: &Visibility) -> Result<Totals, sqlx::Error> {
    let mut qb = QueryBuilder::new(
        "SELECT COUNT(*) AS gifs,
                COALESCE(SUM(CASE WHEN g.tag_count = 0 THEN 1 ELSE 0 END), 0) AS unsorted,
                COALESCE(SUM(CASE WHEN g.status = 'ready' THEN g.bytes_stored ELSE 0 END), 0) AS bytes_stored,
                COALESCE(SUM(CASE WHEN g.status = 'ready' THEN g.bytes_original ELSE 0 END), 0) AS bytes_original
         FROM gifs g WHERE g.deleted_at IS NULL AND g.status IN ('ready', 'processing')",
    );
    vis.push_clause(&mut qb, "g");
    qb.build_query_as().fetch_one(db).await
}

pub async fn trash_count(db: &Db, vis: &Visibility) -> Result<i64, sqlx::Error> {
    let mut qb = QueryBuilder::new("SELECT COUNT(*) FROM gifs g WHERE g.deleted_at IS NOT NULL");
    vis.push_clause(&mut qb, "g");
    qb.build_query_scalar().fetch_one(db).await
}
