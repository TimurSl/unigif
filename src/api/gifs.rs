//! `/gifs`: quick add, list and search, edit, trash, usage, the unsorted queue and batch edits.

use std::time::Duration;

use axum::extract::{FromRequest, Multipart, Path, RawQuery, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::extract::{ApiJson, ApiQuery, TagInput};
use super::types::{GifObject, Page};
use crate::app::App;
use crate::auth::{Auth, Principal, Scope};
use crate::db::gifs::{self, GifRow, ListParams, Sort, Trash};
use crate::db::now_ms;
use crate::error::ApiError;
use crate::ingest::{self, AddRequest, Upload};
use crate::tags::{self, AddTagsError};
use crate::{fetch, media};

const MAX_LIMIT: i64 = 100;
const MAX_BATCH: usize = 500;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn not_found() -> ApiError {
    ApiError::NotFound("no such GIF")
}

fn valid(id: &str) -> Result<(), ApiError> {
    if media::valid_id(id) {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// Build API objects for a page of rows with two batched queries.
pub async fn objects(app: &App, rows: Vec<GifRow>) -> Result<Vec<GifObject>, ApiError> {
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut tags = gifs::tags_for(&app.db, &ids).await?;
    let mut variants = gifs::variants_for(&app.db, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let t = tags.remove(&r.id).unwrap_or_default();
            let v = variants.remove(&r.id).unwrap_or_default();
            GifObject::build(&app.cfg, r, t, &v)
        })
        .collect())
}

async fn object(app: &App, row: GifRow) -> Result<GifObject, ApiError> {
    Ok(objects(app, vec![row]).await?.remove(0))
}

fn map_tag_error(e: AddTagsError) -> ApiError {
    match e {
        AddTagsError::Tag(t) => ApiError::BadRequest(t.to_string()),
        AddTagsError::Db(d) => d.into(),
    }
}

/// Refuse hashtags the key is not allowed to see: tagging with one would make
/// the GIF vanish from the key's own view.
fn check_not_denied(p: &Principal, tags: &[String]) -> Result<(), ApiError> {
    if tags.iter().any(|t| p.deny_tags.contains(t)) {
        return Err(ApiError::Forbidden("this key may not use that hashtag".into()));
    }
    Ok(())
}

fn check_description(d: &Option<String>) -> Result<(), ApiError> {
    if d.as_deref().is_some_and(|d| d.len() > 2000) {
        return Err(ApiError::BadRequest("description is longer than 2000 bytes".into()));
    }
    Ok(())
}

async fn visible(app: &App, p: &Principal, id: &str, trash: Trash) -> Result<GifRow, ApiError> {
    valid(id)?;
    gifs::get_visible(&app.db, id, &p.visibility(), trash).await?.ok_or_else(not_found)
}

// ---------------------------------------------------------------------------
// Quick add
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct AddFields {
    /// Hashtags: separated by commas or spaces in a query string or form field.
    tags: Option<String>,
    source: Option<String>,
    source_id: Option<String>,
    description: Option<String>,
    /// Seconds to wait for processing; capped at 60.
    wait: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct UrlBody {
    url: String,
    tags: Option<TagInput>,
    source: Option<String>,
    source_id: Option<String>,
    description: Option<String>,
    wait: Option<u64>,
}

/// `POST /gifs`: raw body, multipart, or `{"url": …}`.
pub async fn quick_add(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(q): ApiQuery<AddFields>,
    req: Request,
) -> Result<Response, ApiError> {
    p.require(Scope::Upload)?;
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let limit = app.cfg.upload.max_size.0;
    if let Some(len) = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        // Multipart adds a little framing overhead; the streaming check is exact.
        if len > limit + 1024 * 1024 {
            return Err(ApiError::TooLarge { limit });
        }
    }

    let mut fields = q;
    let mut source_url = None;
    let mut tag_list: Option<Vec<String>> = None;
    let upload: Upload;
    if content_type.starts_with("multipart/form-data") {
        let mut mp =
            Multipart::from_request(req, &app).await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
        let mut file: Option<Upload> = None;
        while let Some(mut field) = mp.next_field().await.map_err(|e| ApiError::BadRequest(e.to_string()))? {
            let name = field.name().unwrap_or("").to_string();
            if name == "file" {
                if file.is_some() {
                    return Err(ApiError::BadRequest("send exactly one file field".into()));
                }
                file = Some(ingest::receive(&app, &mut field).await?);
            } else {
                let text = field.text().await.map_err(|e| ApiError::BadRequest(e.to_string()))?;
                match name.as_str() {
                    "tags" => fields.tags = Some(text),
                    "source" => fields.source = Some(text),
                    "source_id" => fields.source_id = Some(text),
                    "description" => fields.description = Some(text),
                    "wait" => fields.wait = text.trim().parse().ok(),
                    _ => {}
                }
            }
        }
        upload = file.ok_or_else(|| ApiError::BadRequest("missing multipart field `file`".into()))?;
    } else if content_type.starts_with("application/json") {
        let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024)
            .await
            .map_err(|e| ApiError::BadRequest(format!("reading request body: {e}")))?;
        let body: UrlBody =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::BadRequest(e.to_string()))?;
        if let Some(t) = &body.tags {
            tag_list = Some(t.normalise()?);
        }
        fields.source = body.source.or(fields.source);
        fields.source_id = body.source_id.or(fields.source_id);
        fields.description = body.description.or(fields.description);
        fields.wait = body.wait.or(fields.wait);
        if fields.source.is_none() {
            fields.source = Some("url".into());
        }
        upload = fetch::fetch(&app, &body.url).await?;
        source_url = Some(body.url);
    } else {
        upload = ingest::receive(&app, req.into_body().into_data_stream()).await?;
    }

    let default_source = if p.key_id().is_some() { "api" } else { "web" };
    let mut add = AddRequest::new(fields.source.as_deref().unwrap_or(default_source));
    add.tags = match tag_list {
        Some(t) => t,
        None => tags::parse_list(fields.tags.as_deref().unwrap_or(""))
            .map_err(|e| ApiError::BadRequest(e.to_string()))?,
    };
    check_not_denied(&p, &add.tags)?;
    add.source_id = fields.source_id.filter(|s| !s.is_empty());
    add.source_url = source_url;
    add.description = fields.description.filter(|s| !s.is_empty());
    add.added_by_key = p.key_id().map(str::to_string);
    add.wait = Duration::from_secs(fields.wait.unwrap_or(app.cfg.upload.wait_secs).min(60));
    let sent_tags = add.tags.clone();

    let added = ingest::add(&app, upload, add).await?;
    let id = added.gif.id.clone();
    let hidden = p.visibility().hides(&app.db, &id).await?;
    let mut obj = object(&app, added.gif).await?;
    // An uploader that may not browse must not learn the library's hashtags
    // through a duplicate hit.
    if !p.has(Scope::Read) || hidden {
        obj.redact(&sent_tags);
    }
    if added.duplicate {
        obj.duplicate = Some(true);
        return Ok((StatusCode::OK, Json(obj)).into_response());
    }
    let mut resp = (StatusCode::CREATED, Json(obj)).into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("/api/v1/gifs/{id}")) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    Ok(resp)
}

// ---------------------------------------------------------------------------
// List and search
// ---------------------------------------------------------------------------

/// Turn `q` into filters: `cat funny` needs both hashtags, `-dog` excludes one,
/// and the last word is a prefix while the user is still typing it.
fn apply_search(q: &str, params: &mut ListParams) {
    let typing = !q.ends_with(|c: char| c.is_whitespace() || c == ',');
    let tokens: Vec<&str> =
        q.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
    for (i, tok) in tokens.iter().enumerate() {
        if let Some(neg) = tok.strip_prefix('-').filter(|n| !n.is_empty()) {
            if let Ok(n) = tags::normalize(neg) {
                params.exclude.push(n);
            }
            continue;
        }
        match tags::normalize(tok) {
            Ok(n) if i == tokens.len() - 1 && typing => params.prefix = Some(n),
            Ok(n) => params.tags.push(n),
            Err(tags::TagError::Empty(_)) => {}
            Err(_) => params.impossible = true,
        }
    }
}

fn parse_bool(v: &str) -> Result<bool, ApiError> {
    match v {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ApiError::BadRequest(format!("expected true or false, got {v:?}"))),
    }
}

fn parse_statuses(v: &str) -> Result<Vec<String>, ApiError> {
    let list: Vec<String> = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if list.is_empty() || list.iter().any(|s| !matches!(s.as_str(), "ready" | "processing" | "failed")) {
        return Err(ApiError::BadRequest("status must list ready, processing and/or failed".into()));
    }
    Ok(list)
}

fn default_statuses() -> Vec<String> {
    vec!["ready".into(), "processing".into()]
}

pub async fn list(
    State(app): State<App>,
    Auth(p): Auth,
    RawQuery(raw): RawQuery,
) -> Result<Json<Page<GifObject>>, ApiError> {
    p.require(Scope::Read)?;
    let pairs: Vec<(String, String)> = serde_urlencoded::from_str(raw.as_deref().unwrap_or(""))
        .map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))?;
    let mut params = ListParams { limit: 50, statuses: default_statuses(), ..Default::default() };
    for (k, v) in &pairs {
        match k.as_str() {
            "q" => apply_search(v, &mut params),
            "tag" => match tags::normalize(v) {
                Ok(n) => params.tags.push(n),
                Err(_) => params.impossible = true,
            },
            "unsorted" => params.unsorted = parse_bool(v)?,
            "trash" => params.trash = parse_bool(v)?,
            "sort" => {
                params.sort = Sort::parse(v)
                    .ok_or_else(|| ApiError::BadRequest("sort must be new, old or used".into()))?
            }
            "status" => params.statuses = parse_statuses(v)?,
            "limit" => {
                params.limit = v
                    .parse::<i64>()
                    .map_err(|_| ApiError::BadRequest("limit must be a number".into()))?
                    .clamp(1, MAX_LIMIT)
            }
            "cursor" => {
                params.cursor = Some(
                    gifs::Cursor::decode(v).ok_or_else(|| ApiError::BadRequest("invalid cursor".into()))?,
                )
            }
            _ => {}
        }
    }
    if params.trash {
        p.require(Scope::Delete)?;
    }
    let (rows, next_cursor) = gifs::list(&app.db, &params, &p.visibility()).await?;
    Ok(Json(Page { items: objects(&app, rows).await?, next_cursor }))
}

pub async fn get(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<Json<GifObject>, ApiError> {
    p.require(Scope::Read)?;
    let row = visible(&app, &p, &id, Trash::Without).await?;
    Ok(Json(object(&app, row).await?))
}

#[derive(Deserialize)]
pub struct NextQuery {
    after: Option<String>,
    sort: Option<String>,
}

/// The next unsorted GIF after `after` (which may already have been sorted).
pub async fn unsorted_next(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(q): ApiQuery<NextQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require(Scope::Read)?;
    let sort = match q.sort.as_deref() {
        None => Sort::New,
        Some(s) => match Sort::parse(s) {
            Some(s @ (Sort::New | Sort::Old)) => s,
            _ => return Err(ApiError::BadRequest("sort must be new or old".into())),
        },
    };
    let vis = p.visibility();
    let mut params =
        ListParams { unsorted: true, sort, limit: 1, statuses: default_statuses(), ..Default::default() };
    let total = gifs::count(&app.db, &params, &vis).await?;
    if let Some(after) = q.after.as_deref().filter(|a| media::valid_id(a)) {
        if let Some(row) = gifs::get(&app.db, after).await? {
            params.cursor = Some(gifs::cursor_after(&row, sort));
        }
    }
    let (rows, _) = gifs::list(&app.db, &params, &vis).await?;
    let remaining = (gifs::count(&app.db, &params, &vis).await? - rows.len() as i64).max(0);
    let item = match rows.into_iter().next() {
        Some(r) => Some(object(&app, r).await?),
        None => None,
    };
    Ok(Json(json!({ "item": item, "remaining": remaining, "total": total })))
}

// ---------------------------------------------------------------------------
// Edit, tag, trash, use
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PatchBody {
    tags: Option<TagInput>,
    #[serde(default, deserialize_with = "super::extract::double_option")]
    description: Option<Option<String>>,
}

pub async fn patch(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<PatchBody>,
) -> Result<Json<GifObject>, ApiError> {
    p.require(Scope::Tag)?;
    if b.tags.is_none() && b.description.is_none() {
        return Err(ApiError::BadRequest("send tags and/or description".into()));
    }
    let row = visible(&app, &p, &id, Trash::Without).await?;
    let new_tags = b.tags.as_ref().map(TagInput::normalise).transpose()?;
    if let Some(t) = &new_tags {
        check_not_denied(&p, t)?;
    }
    if let Some(Some(d)) = &b.description {
        check_description(&Some(d.clone()))?;
    }
    let now = now_ms();
    let mut tx = crate::db::begin_write(&app.db).await?;
    if let Some(t) = &new_tags {
        tags::replace_tags(&mut tx, &row.id, t, now).await.map_err(map_tag_error)?;
    }
    if let Some(d) = &b.description {
        let d = d.as_deref().filter(|s| !s.is_empty());
        sqlx::query("UPDATE gifs SET description = ? WHERE id = ?")
            .bind(d)
            .bind(&row.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    app.emit("gif.tagged", &row.id);
    let row = gifs::get(&app.db, &row.id).await?.ok_or_else(not_found)?;
    Ok(Json(object(&app, row).await?))
}

#[derive(Deserialize)]
pub struct TagsBody {
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

pub async fn edit_tags(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<TagsBody>,
) -> Result<Json<GifObject>, ApiError> {
    p.require(Scope::Tag)?;
    let row = visible(&app, &p, &id, Trash::Without).await?;
    let add = tags::normalize_all(&b.add).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let remove = tags::normalize_all(&b.remove).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    check_not_denied(&p, &add)?;
    let now = now_ms();
    let mut tx = crate::db::begin_write(&app.db).await?;
    tags::remove_tags(&mut tx, &row.id, &remove).await?;
    tags::add_tags(&mut tx, &row.id, &add, now).await.map_err(map_tag_error)?;
    tx.commit().await?;
    app.emit("gif.tagged", &row.id);
    let row = gifs::get(&app.db, &row.id).await?.ok_or_else(not_found)?;
    Ok(Json(object(&app, row).await?))
}

pub async fn trash(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    p.require(Scope::Delete)?;
    let row = visible(&app, &p, &id, Trash::Without).await?;
    let mut tx = crate::db::begin_write(&app.db).await?;
    gifs::trash(&mut tx, &row.id, now_ms()).await?;
    tx.commit().await?;
    app.emit("gif.deleted", &row.id);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn restore(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<Json<GifObject>, ApiError> {
    p.require(Scope::Delete)?;
    let row = visible(&app, &p, &id, Trash::Only).await?;
    let mut tx = crate::db::begin_write(&app.db).await?;
    gifs::restore(&mut tx, &row.id).await?;
    tx.commit().await?;
    app.emit("gif.added", &row.id);
    let row = gifs::get(&app.db, &row.id).await?.ok_or_else(not_found)?;
    Ok(Json(object(&app, row).await?))
}

/// `POST /gifs/trash/empty`: purge everything in the trash the caller can
/// see, files and cached conversions included.
pub async fn empty_trash(State(app): State<App>, Auth(p): Auth) -> Result<Json<serde_json::Value>, ApiError> {
    p.require(Scope::Delete)?;
    let purged = crate::janitor::empty_trash(&app, &p.visibility()).await?;
    Ok(Json(json!({ "purged": purged })))
}

/// A GIF was sent somewhere: feeds `sort=used`.
pub async fn use_gif(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    p.require(Scope::Read)?;
    let row = visible(&app, &p, &id, Trash::Without).await?;
    sqlx::query("UPDATE gifs SET use_count = use_count + 1, last_used_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(&row.id)
        .execute(&app.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Batch
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct BatchBody {
    ids: Vec<String>,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    trash: bool,
}

pub async fn batch(
    State(app): State<App>,
    Auth(p): Auth,
    ApiJson(b): ApiJson<BatchBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tagging = !b.add.is_empty() || !b.remove.is_empty();
    if !tagging && !b.trash {
        return Err(ApiError::BadRequest("send add, remove and/or trash".into()));
    }
    if tagging {
        p.require(Scope::Tag)?;
    }
    if b.trash {
        p.require(Scope::Delete)?;
    }
    if b.ids.is_empty() || b.ids.len() > MAX_BATCH {
        return Err(ApiError::BadRequest(format!("ids must hold 1 to {MAX_BATCH} entries")));
    }
    let add = tags::normalize_all(&b.add).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let remove = tags::normalize_all(&b.remove).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    check_not_denied(&p, &add)?;

    let vis = p.visibility();
    let now = now_ms();
    let mut seen = std::collections::HashSet::new();
    let (mut updated, mut missing) = (Vec::new(), Vec::new());
    // One transaction: either every visible GIF is changed or none is.
    let mut tx = crate::db::begin_write(&app.db).await?;
    for id in b.ids.iter().filter(|i| seen.insert((*i).clone())) {
        let found = if media::valid_id(id) {
            gifs::get_visible(&app.db, id, &vis, Trash::Without).await?
        } else {
            None
        };
        let Some(row) = found else {
            missing.push(id.clone());
            continue;
        };
        if tagging {
            tags::remove_tags(&mut tx, &row.id, &remove).await?;
            tags::add_tags(&mut tx, &row.id, &add, now).await.map_err(map_tag_error)?;
        }
        if b.trash {
            gifs::trash(&mut tx, &row.id, now).await?;
        }
        updated.push(row.id);
    }
    tx.commit().await?;
    for id in &updated {
        app.emit(if b.trash { "gif.deleted" } else { "gif.tagged" }, id);
    }
    Ok(Json(json!({ "updated": updated.len(), "not_found": missing })))
}
