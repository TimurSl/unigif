//! HTTP router.

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderName, HeaderValue, Method};
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::app::App;
use crate::error::ApiError;
use crate::{auth, web};

mod extract;
pub mod files;
mod gifs;
mod keys;
mod misc;
mod search;
mod session;
mod share;
mod tags;
pub mod types;

pub use misc::spec;

const CSP: &str = "default-src 'self'; img-src 'self' data: blob: https:; media-src 'self' blob: https:; \
style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; \
frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn cors(app: &App) -> CorsLayer {
    let origins = &app.cfg.cors.origins;
    let allow = if origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(origins.iter().filter_map(|o| HeaderValue::from_str(o.trim_end_matches('/')).ok()))
    };
    // No `allow_credentials`: browsers will not attach the session cookie to
    // cross-origin requests, so CORS only ever serves key-authenticated calls.
    CorsLayer::new()
        .allow_origin(allow)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-api-key"),
            HeaderName::from_static("x-unigif-request"),
        ])
        .expose_headers([
            header::LOCATION,
            header::RETRY_AFTER,
            HeaderName::from_static("ratelimit-limit"),
            HeaderName::from_static("ratelimit-remaining"),
            HeaderName::from_static("ratelimit-reset"),
        ])
        .max_age(std::time::Duration::from_secs(3600))
}

pub fn router(app: App) -> Router {
    let upload_limit = app.cfg.upload.max_size.0 as usize + 1024 * 1024;

    let api = Router::new()
        .route("/auth/login", post(session::login))
        .route("/auth/logout", post(session::logout))
        .route("/auth/password", post(session::change_password))
        .route("/auth/sessions", get(session::list_sessions))
        .route("/auth/sessions/{id}", delete(session::revoke_session))
        .route("/me", get(session::me).patch(session::patch_me))
        .route("/gifs", get(gifs::list).post(gifs::quick_add).layer(DefaultBodyLimit::max(upload_limit)))
        .route("/gifs/unsorted/next", get(gifs::unsorted_next))
        .route("/gifs/batch", post(gifs::batch))
        .route("/gifs/trash/empty", post(gifs::empty_trash))
        .route("/gifs/{id}", get(gifs::get).patch(gifs::patch).delete(gifs::trash))
        .route("/gifs/{id}/tags", post(gifs::edit_tags))
        .route("/gifs/{id}/restore", post(gifs::restore))
        .route("/gifs/{id}/use", post(gifs::use_gif))
        .route("/gifs/{id}/convert", post(gifs::convert))
        .route("/tags", get(tags::list))
        .route("/tags/suggest", get(tags::suggest))
        .route("/tags/{name}", patch(tags::rename).delete(tags::delete))
        .route("/tags/{name}/merge", post(tags::merge))
        .route("/search", get(search::find))
        .route("/search/providers", get(search::providers))
        .route("/search/providers/{id}/key", put(search::set_key))
        .route("/keys", get(keys::list).post(keys::create))
        .route("/keys/{id}", patch(keys::patch).delete(keys::revoke))
        .route("/stats", get(misc::stats))
        .route("/cache", delete(misc::clear_cache))
        .route("/events", get(misc::events))
        .route("/openapi.json", get(misc::openapi_json))
        .fallback(|| async { ApiError::NotFound("no such endpoint") })
        .layer(from_fn_with_state(app.clone(), auth::middleware))
        .layer(cors(&app));

    let header = |name: &'static str, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(HeaderName::from_static(name), HeaderValue::from_static(value))
    };

    Router::new()
        .nest("/api/v1", api)
        .route("/api/docs", get(misc::docs))
        .route("/m/{id}/{variant}", get(files::serve))
        .route("/g/{slug}", get(share::page))
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

async fn readyz(axum::extract::State(app): axum::extract::State<App>) -> Result<&'static str, ApiError> {
    sqlx::query_scalar::<_, i64>("SELECT 1").fetch_one(&app.db).await?;
    Ok("ready")
}
