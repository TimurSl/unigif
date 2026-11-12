//! `GET /g/{id}`: a small public page with OpenGraph / Twitter-card tags, so a
//! pasted link unfurls in chats; and `/g/{id}.mp4|gif|webm|webp`, which
//! redirect to the media file.
//!
//! Like media, the page is public to anyone who knows the (unguessable) id,
//! unless `media.public = false`, which puts both behind a login or key.
//! It deliberately shows no hashtags or description: those are the owner's
//! private organisation, not part of what is being shared.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Redirect, Response};

use crate::app::App;
use crate::db::gifs;
use crate::error::ApiError;
use crate::media::{self, Variant};

/// Extensions accepted after the id, and the variant each one redirects to.
const REDIRECTS: [(&str, Variant); 4] = [
    ("mp4", Variant::FullMp4),
    ("gif", Variant::FullGif),
    ("webm", Variant::FullWebm),
    ("webp", Variant::FullWebp),
];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Absolute base URL for the tags: the configured public URL, or what the
/// request itself says (Open Graph consumers need absolute URLs).
fn origin(app: &App, headers: &HeaderMap) -> String {
    if !app.cfg.base_url().is_empty() {
        return app.cfg.base_url().to_string();
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "https" || *p == "http")
        .unwrap_or("http");
    format!("{proto}://{host}")
}

pub async fn page(
    State(app): State<App>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (id, ext) = match slug.split_once('.') {
        Some((id, ext)) => (id, Some(ext)),
        None => (slug.as_str(), None),
    };
    if !media::valid_id(id) {
        return Err(ApiError::NotFound("no such gif"));
    }
    let row = gifs::get(&app.db, id).await?.ok_or(ApiError::NotFound("no such gif"))?;
    if row.status != "ready" || row.deleted_at.is_some() {
        return Err(ApiError::NotFound("no such gif"));
    }
    crate::auth::check_media_access(&app, &headers, &row.id).await?;

    if let Some(ext) = ext {
        let variant = REDIRECTS
            .iter()
            .find(|(e, _)| e.eq_ignore_ascii_case(ext))
            .map(|(_, v)| *v)
            .ok_or(ApiError::NotFound("no such gif"))?;
        // Temporary: a GIF can be trashed, and the target is cheap to resolve.
        return Ok(Redirect::temporary(&format!("/m/{id}/{}", variant.file_name())).into_response());
    }

    let base = origin(&app, &headers);
    let page_url = format!("{base}/g/{id}");
    let mp4 = format!("{base}/m/{id}/full.mp4");
    let poster = format!("{base}/m/{id}/poster.webp");
    let (w, h) = (row.width.unwrap_or(0), row.height.unwrap_or(0));
    let variants = gifs::variants_of(&app.db, id).await?;
    let (pw, ph) = variants
        .iter()
        .find(|v| v.name == "poster.webp")
        .map(|v| (v.width.unwrap_or(0), v.height.unwrap_or(0)))
        .unwrap_or((0, 0));

    let mut head = String::new();
    let mut meta = |kind: &str, name: &str, value: &str| {
        head.push_str(&format!("<meta {kind}=\"{}\" content=\"{}\">\n", esc(name), esc(value)));
    };
    meta("property", "og:site_name", "Unigif");
    meta("property", "og:type", "video.other");
    meta("property", "og:title", "GIF");
    meta("property", "og:url", &page_url);
    meta("property", "og:image", &poster);
    meta("property", "og:image:type", "image/webp");
    meta("property", "og:video", &mp4);
    if mp4.starts_with("https://") {
        meta("property", "og:video:secure_url", &mp4);
    }
    meta("property", "og:video:type", "video/mp4");
    if w > 0 && h > 0 {
        meta("property", "og:video:width", &w.to_string());
        meta("property", "og:video:height", &h.to_string());
    }
    if pw > 0 && ph > 0 {
        meta("property", "og:image:width", &pw.to_string());
        meta("property", "og:image:height", &ph.to_string());
    }
    meta("name", "twitter:card", "summary_large_image");
    meta("name", "twitter:title", "GIF");
    meta("name", "twitter:image", &poster);
    meta("name", "robots", "noindex");

    let size = if w > 0 && h > 0 { format!(" width=\"{w}\" height=\"{h}\"") } else { String::new() };
    let body = format!(
        "<!doctype html>\n<html lang=\"en\">\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<title>GIF</title>\n{head}\
<style>\
html{{color-scheme:dark}}\
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0a0a0a;color:#fafafa;font:14px/1.5 system-ui,sans-serif}}\
main{{display:grid;gap:12px;justify-items:center;padding:16px;max-width:100vw;box-sizing:border-box}}\
video{{max-width:100%;max-height:78vh;height:auto;border-radius:10px;background:#111}}\
nav{{display:flex;gap:16px;flex-wrap:wrap;justify-content:center}}\
a{{color:#a1a1a1}}a:hover{{color:#fafafa}}\
</style>\n\
<body><main>\n\
<video src=\"/m/{id}/full.mp4\" poster=\"/m/{id}/poster.webp\"{size} autoplay loop muted playsinline controls></video>\n\
<nav><a href=\"/g/{id}.mp4\">MP4</a><a href=\"/g/{id}.gif\">GIF</a><a href=\"/g/{id}.webm\">WebM</a><a href=\"/g/{id}.webp\">WebP</a></nav>\n\
</main></body></html>\n"
    );

    let mut resp = body.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    // Link unfurlers fetch this once; a short cache keeps a trashed GIF from lingering.
    let cache_control = if app.cfg.media.public { "public, max-age=60" } else { "private, max-age=60" };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::esc;

    #[test]
    fn escapes_html() {
        assert_eq!(esc("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
    }
}
