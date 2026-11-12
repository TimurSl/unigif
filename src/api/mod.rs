//! HTTP router and the milestone-1 handlers.

use std::time::Duration;

use axum::extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::app::App;
use crate::db::gifs;
use crate::error::ApiError;
use crate::ingest::{self, AddRequest, Upload};
use crate::{media, tags, web};

mod auth;
pub mod files;
pub mod types;

use auth::Authed;
use types::GifObject;

const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; media-src 'self' blob:; \
style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub fn router(app: App) -> Router {
    let upload_limit = app.cfg.upload.max_size.0 as usize + 1024 * 1024;

    let api = Router::new()
        .route("/gifs", post(quick_add).layer(DefaultBodyLimit::max(upload_limit)))
        .route("/gifs/{id}", get(get_gif));

    let header = |name: &'static str, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(HeaderName::from_static(name), HeaderValue::from_static(value))
    };

    Router::new()
        .nest("/api/v1", api)
        .route("/m/{id}/{variant}", get(files::serve))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .fallback(web::handler)
        .layer(CompressionLayer::new().gzip(true).no_br().no_deflate().no_zstd())
        .layer(header("x-content-type-options", "nosniff"))
        .layer(header("referrer-policy", "no-referrer"))
        .layer(header("content-security-policy", CSP))
        .layer(TraceLayer::new_for_http())
        .with_state(app)
}

async fn readyz(State(app): State<App>) -> Result<&'static str, ApiError> {
    sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&app.db).await?;
    Ok("ready")
}

#[derive(Debug, Default, Deserialize)]
struct AddQuery {
    /// Hashtags separated by commas or spaces.
    tags: Option<String>,
    source: Option<String>,
    source_id: Option<String>,
    description: Option<String>,
    /// Seconds to wait for processing; capped at 60.
    wait: Option<u64>,
}

/// `POST /api/v1/gifs`: quick add (raw body or multipart; URL bodies come later).
async fn quick_add(
    State(app): State<App>,
    _auth: Authed,
    Query(q): Query<AddQuery>,
    req: Request,
) -> Result<Response, ApiError> {
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
        // Multipart adds a little framing overhead; the streaming check below is exact.
        if len > limit + 1024 * 1024 {
            return Err(ApiError::TooLarge { limit });
        }
    }

    let mut fields = q;
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
        return Err(ApiError::NotImplemented("adding by URL is not available yet; send the file bytes"));
    } else {
        upload = ingest::receive(&app, req.into_body().into_data_stream()).await?;
    }

    let mut add = AddRequest::new(fields.source.as_deref().unwrap_or("api"));
    add.tags = tags::parse_list(fields.tags.as_deref().unwrap_or(""))
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    add.source_id = fields.source_id.filter(|s| !s.is_empty());
    add.description = fields.description.filter(|s| !s.is_empty());
    add.wait = Duration::from_secs(fields.wait.unwrap_or(app.cfg.upload.wait_secs).min(60));

    let added = ingest::add(&app, upload, add).await?;
    let id = added.gif.id.clone();
    let mut obj = load_object(&app, added.gif).await?;
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

/// `GET /api/v1/gifs/{id}`
async fn get_gif(
    State(app): State<App>,
    _auth: Authed,
    Path(id): Path<String>,
) -> Result<Json<GifObject>, ApiError> {
    if !media::valid_id(&id) {
        return Err(ApiError::NotFound("no such GIF"));
    }
    let row = gifs::get(&app.db, &id).await?.ok_or(ApiError::NotFound("no such GIF"))?;
    if row.deleted_at.is_some() {
        return Err(ApiError::NotFound("no such GIF"));
    }
    Ok(Json(load_object(&app, row).await?))
}

async fn load_object(app: &App, row: gifs::GifRow) -> Result<GifObject, ApiError> {
    let tags = gifs::tags_of(&app.db, &row.id).await?;
    let variants = gifs::variants_of(&app.db, &row.id).await?;
    Ok(GifObject::build(&app.cfg, row, tags, &variants))
}
