//! `/tags`: listing, suggestions, rename, merge, delete.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sqlx::{FromRow, QueryBuilder, Sqlite};

use super::extract::{ApiJson, ApiQuery};
use super::types::{rfc3339, Page, TagObject};
use crate::app::App;
use crate::auth::{Auth, Principal, Scope};
use crate::db::gifs::Visibility;
use crate::db::{now_ms, Db};
use crate::error::ApiError;
use crate::tags::{self, TagOpError};

#[derive(Debug, Clone, Copy)]
enum Order {
    Recent,
    Popular,
    Alpha,
}

#[derive(FromRow)]
struct TagRow {
    name: String,
    gif_count: i64,
    last_used_at: Option<i64>,
}

impl From<TagRow> for TagObject {
    fn from(r: TagRow) -> Self {
        TagObject { name: r.name, gif_count: r.gif_count, last_used_at: r.last_used_at.map(rfc3339) }
    }
}

/// Tags with live GIFs on them. For a key with hidden hashtags the counts are
/// taken over the GIFs it can see, so nothing leaks through them.
async fn query_tags(
    db: &Db,
    vis: &Visibility,
    prefix: Option<&str>,
    exclude: &[String],
    order: Order,
    limit: i64,
    offset: i64,
) -> Result<Vec<TagRow>, sqlx::Error> {
    // The maintained `tags.gif_count` counts GIFs outside the trash. With
    // hidden hashtags, what the caller cannot see is subtracted; counting
    // through gif_tags instead would cost a pass over the whole library.
    let mut qb: QueryBuilder<Sqlite> = if vis.is_all() {
        QueryBuilder::new(
            "SELECT t.name AS name, t.gif_count AS gif_count, t.last_used_at AS last_used_at
             FROM tags t WHERE t.gif_count > 0",
        )
    } else {
        let mut qb = QueryBuilder::new(
            "SELECT t.name AS name, t.gif_count - COALESCE(h.n, 0) AS gif_count, t.last_used_at AS last_used_at
             FROM tags t LEFT JOIN ",
        );
        vis.push_hidden_tag_counts(&mut qb);
        qb.push(" h ON h.tag_id = t.id WHERE t.gif_count - COALESCE(h.n, 0) > 0");
        qb
    };
    if let Some(p) = prefix {
        qb.push(" AND t.name >= ").push_bind(p.to_string());
        qb.push(" AND t.name <= ").push_bind(format!("{p}\u{10FFFF}"));
    }
    if !exclude.is_empty() {
        qb.push(" AND t.name NOT IN (");
        let mut sep = qb.separated(", ");
        for e in exclude {
            sep.push_bind(e.clone());
        }
        qb.push(")");
    }
    qb.push(match order {
        Order::Recent => " ORDER BY t.last_used_at DESC, t.name",
        Order::Popular => " ORDER BY gif_count DESC, t.name",
        Order::Alpha => " ORDER BY t.name",
    });
    qb.push(" LIMIT ").push_bind(limit).push(" OFFSET ").push_bind(offset);
    qb.build_query_as().fetch_all(db).await
}

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    sort: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

fn decode_offset(c: &str) -> Option<i64> {
    String::from_utf8(URL_SAFE_NO_PAD.decode(c).ok()?).ok()?.parse().ok().filter(|n: &i64| *n >= 0)
}

pub async fn list(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> Result<Json<Page<TagObject>>, ApiError> {
    p.require(Scope::Read)?;
    let order = match q.sort.as_deref().unwrap_or("alpha") {
        "recent" => Order::Recent,
        "popular" => Order::Popular,
        "alpha" => Order::Alpha,
        _ => return Err(ApiError::BadRequest("sort must be recent, popular or alpha".into())),
    };
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let offset = match &q.cursor {
        Some(c) => decode_offset(c).ok_or_else(|| ApiError::BadRequest("invalid cursor".into()))?,
        None => 0,
    };
    // A prefix that cannot be a hashtag matches nothing.
    let prefix = match q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => match tags::normalize(s) {
            Ok(n) => Some(n),
            Err(_) => return Ok(Json(Page { items: vec![], next_cursor: None })),
        },
        None => None,
    };
    let mut rows =
        query_tags(&app.db, &p.visibility(), prefix.as_deref(), &[], order, limit + 1, offset).await?;
    let next_cursor = (rows.len() as i64 > limit).then(|| {
        rows.truncate(limit as usize);
        URL_SAFE_NO_PAD.encode((offset + limit).to_string())
    });
    Ok(Json(Page { items: rows.into_iter().map(Into::into).collect(), next_cursor }))
}

#[derive(Deserialize)]
pub struct SuggestQuery {
    selected: Option<String>,
    limit: Option<i64>,
}

/// Recent hashtags, plus the ones that most often appear next to the selected ones.
pub async fn suggest(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(q): ApiQuery<SuggestQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require(Scope::Read)?;
    let limit = q.limit.unwrap_or(12).clamp(1, 50);
    let selected: Vec<String> = q
        .selected
        .as_deref()
        .map(|s| {
            s.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|t| tags::normalize(t).ok()).collect()
        })
        .unwrap_or_default();
    let vis = p.visibility();
    let recent = query_tags(&app.db, &vis, None, &selected, Order::Recent, limit, 0).await?;

    let mut related = Vec::new();
    if !selected.is_empty() {
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT t.name AS name, COUNT(*) AS gif_count, t.last_used_at AS last_used_at
             FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id JOIN gifs g ON g.id = gt.gif_id
             WHERE g.deleted_at IS NULL AND gt.gif_id IN (
               SELECT s.gif_id FROM gif_tags s JOIN tags st ON st.id = s.tag_id WHERE st.name IN (",
        );
        let mut sep = qb.separated(", ");
        for s in &selected {
            sep.push_bind(s.clone());
        }
        qb.push(") GROUP BY s.gif_id HAVING COUNT(*) = ").push_bind(selected.len() as i64);
        qb.push(") AND t.name NOT IN (");
        let mut sep = qb.separated(", ");
        for s in &selected {
            sep.push_bind(s.clone());
        }
        qb.push(")");
        vis.push_clause(&mut qb, "g");
        qb.push(" GROUP BY t.id ORDER BY gif_count DESC, t.name LIMIT ").push_bind(limit);
        related = qb.build_query_as::<TagRow>().fetch_all(&app.db).await?;
    }
    Ok(Json(serde_json::json!({
        "recent": recent.into_iter().map(TagObject::from).collect::<Vec<_>>(),
        "related": related.into_iter().map(TagObject::from).collect::<Vec<_>>(),
    })))
}

/// Renaming, merging and deleting touch every GIF, including ones a key with
/// hidden hashtags cannot see, so those keys may not do it.
fn require_whole_library(p: &Principal) -> Result<(), ApiError> {
    p.require(Scope::Tag)?;
    if !p.deny_tags.is_empty() {
        return Err(ApiError::Forbidden(
            "keys with hidden hashtags cannot change hashtags library-wide".into(),
        ));
    }
    Ok(())
}

fn op_error(e: TagOpError) -> ApiError {
    match e {
        TagOpError::NotFound => ApiError::NotFound("no such hashtag"),
        TagOpError::Exists => ApiError::Conflict(TagOpError::Exists.to_string()),
        TagOpError::Invalid(t) => ApiError::BadRequest(t.to_string()),
        TagOpError::Db(d) => d.into(),
    }
}

fn path_tag(raw: &str) -> Result<String, ApiError> {
    tags::normalize(raw).map_err(|e| ApiError::BadRequest(e.to_string()))
}

async fn tag_object(db: &Db, name: &str) -> Result<TagObject, ApiError> {
    let row: TagRow = sqlx::query_as("SELECT name, gif_count, last_used_at FROM tags WHERE name = ?")
        .bind(name)
        .fetch_optional(db)
        .await?
        .ok_or(ApiError::NotFound("no such hashtag"))?;
    Ok(row.into())
}

fn announce(app: &App, ids: &[String]) {
    for id in ids {
        app.emit("gif.tagged", id);
    }
}

#[derive(Deserialize)]
pub struct RenameBody {
    name: String,
}

pub async fn rename(
    State(app): State<App>,
    Auth(p): Auth,
    Path(name): Path<String>,
    ApiJson(b): ApiJson<RenameBody>,
) -> Result<Json<TagObject>, ApiError> {
    require_whole_library(&p)?;
    let from = path_tag(&name)?;
    let to = path_tag(&b.name)?;
    let mut tx = crate::db::begin_write(&app.db).await?;
    tags::rename(&mut tx, &from, &to).await.map_err(op_error)?;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT gif_id FROM gif_tags gt JOIN tags t ON t.id = gt.tag_id WHERE t.name = ?")
            .bind(&to)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    announce(&app, &ids);
    Ok(Json(tag_object(&app.db, &to).await?))
}

#[derive(Deserialize)]
pub struct MergeBody {
    into: String,
}

pub async fn merge(
    State(app): State<App>,
    Auth(p): Auth,
    Path(name): Path<String>,
    ApiJson(b): ApiJson<MergeBody>,
) -> Result<Json<TagObject>, ApiError> {
    require_whole_library(&p)?;
    let from = path_tag(&name)?;
    let into = path_tag(&b.into)?;
    let mut tx = crate::db::begin_write(&app.db).await?;
    let ids = tags::merge(&mut tx, &from, &into, now_ms()).await.map_err(op_error)?;
    tx.commit().await?;
    announce(&app, &ids);
    Ok(Json(tag_object(&app.db, &into).await?))
}

pub async fn delete(
    State(app): State<App>,
    Auth(p): Auth,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_whole_library(&p)?;
    let name = path_tag(&name)?;
    let mut tx = crate::db::begin_write(&app.db).await?;
    let ids = tags::delete(&mut tx, &name, now_ms()).await.map_err(op_error)?;
    tx.commit().await?;
    announce(&app, &ids);
    Ok(StatusCode::NO_CONTENT)
}

/// Number of hashtags with at least one visible live GIF.
pub async fn count_visible(db: &Db, vis: &Visibility) -> Result<i64, sqlx::Error> {
    if vis.is_all() {
        return sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE gif_count > 0").fetch_one(db).await;
    }
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT COUNT(*) FROM tags t LEFT JOIN ");
    vis.push_hidden_tag_counts(&mut qb);
    qb.push(" h ON h.tag_id = t.id WHERE t.gif_count - COALESCE(h.n, 0) > 0");
    qb.build_query_scalar().fetch_one(db).await
}
