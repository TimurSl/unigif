//! The embedded single-page app (`web/dist`, built by Vite).

use axum::http::{header, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

const PLACEHOLDER: &str = "<!doctype html><meta charset=utf-8>\
<meta name=viewport content='width=device-width,initial-scale=1'>\
<title>unigif</title>\
<body style='background:#0a0a0a;color:#fafafa;font:16px system-ui;display:grid;place-items:center;height:100vh;margin:0'>\
<p>unigif is running. The web UI has not been built into this binary.</p>";

/// Serve a built asset, falling back to `index.html` for client-side routes.
pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") || path.starts_with("m/") || path.starts_with("g/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(resp) = asset(if path.is_empty() { "index.html" } else { path }) {
        return resp;
    }
    // A missing file with an extension is a real 404, not an SPA route.
    if path.rsplit('/').next().is_some_and(|last| last.contains('.')) {
        return StatusCode::NOT_FOUND.into_response();
    }
    asset("index.html").unwrap_or_else(|| Html(PLACEHOLDER).into_response())
}

fn asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_for(path);
    // Vite emits content-hashed files under assets/.
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    Some(
        ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)], file.data.into_owned())
            .into_response(),
    )
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
