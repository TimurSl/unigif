//! `GET /m/{id}/{variant}`: stored media with Range support and immutable caching.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use crate::app::App;
use crate::db::gifs;
use crate::error::ApiError;
use crate::media::{self, Variant};

const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

#[derive(Debug, PartialEq, Eq)]
pub enum RangeSpec {
    /// No usable `Range` header: send everything.
    Full,
    /// Inclusive byte range.
    Partial(u64, u64),
    Unsatisfiable,
}

/// Parse a single-range `Range: bytes=…` header. Multiple ranges are not
/// supported and are answered with the whole file, which RFC 9110 permits.
pub fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(h) = header else { return RangeSpec::Full };
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return RangeSpec::Full };
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let Some((a, b)) = spec.split_once('-') else { return RangeSpec::Full };
    let (a, b) = (a.trim(), b.trim());
    if len == 0 {
        return RangeSpec::Unsatisfiable;
    }
    match (a.is_empty(), b.is_empty()) {
        (true, true) => RangeSpec::Full,
        // suffix: last n bytes
        (true, false) => match b.parse::<u64>() {
            Ok(0) => RangeSpec::Unsatisfiable,
            Ok(n) => RangeSpec::Partial(len.saturating_sub(n), len - 1),
            Err(_) => RangeSpec::Full,
        },
        (false, empty_end) => {
            let Ok(start) = a.parse::<u64>() else { return RangeSpec::Full };
            let end = if empty_end {
                len - 1
            } else {
                match b.parse::<u64>() {
                    Ok(e) => e.min(len - 1),
                    Err(_) => return RangeSpec::Full,
                }
            };
            if start >= len || start > end {
                RangeSpec::Unsatisfiable
            } else {
                RangeSpec::Partial(start, end)
            }
        }
    }
}

/// Open a stored variant, or an on-demand one (converting it first if it is
/// not cached yet).
async fn open_variant(app: &App, row: &gifs::GifRow, variant: Variant) -> Result<tokio::fs::File, ApiError> {
    if variant.is_stored() {
        let path = media::gif_dir(&app.cfg.media_dir(), &row.id).join(variant.file_name());
        return tokio::fs::File::open(&path).await.map_err(|_| ApiError::NotFound("no such media"));
    }
    // A concurrent cache trim can remove the file between the conversion and
    // the open; one more round settles it.
    for _ in 0..2 {
        let path = crate::cache::ensure(app, row, variant).await?;
        if let Ok(f) = tokio::fs::File::open(&path).await {
            return Ok(f);
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!("cached file for {} vanished twice", variant.file_name())))
}

pub async fn serve(
    State(app): State<App>,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // The id and variant name are validated before they touch a path.
    let variant = Variant::parse(&name).ok_or(ApiError::NotFound("no such media"))?;
    if !media::valid_id(&id) {
        return Err(ApiError::NotFound("no such media"));
    }
    let row = gifs::get(&app.db, &id).await?.ok_or(ApiError::NotFound("no such media"))?;
    if row.status != "ready" || row.deleted_at.is_some() {
        return Err(ApiError::NotFound("no such media"));
    }

    let mut file = open_variant(&app, &row, variant).await?;
    let len = file.metadata().await.map_err(anyhow::Error::from)?.len();

    // Files are written once and never modified, so (id, variant, length)
    // identifies the bytes.
    let etag = format!("\"{id}-{}-{len}\"", variant.file_name());
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::ETAG, HeaderValue::from_str(&etag).map_err(anyhow::Error::from)?);
    resp_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL));
    resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp_headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(variant.content_type()));

    let if_none_match = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok());
    if if_none_match.is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*")) {
        return Ok((StatusCode::NOT_MODIFIED, resp_headers).into_response());
    }

    // If-Range: only honour the Range when the validator still matches.
    let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let honour_range =
        headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).map(|v| v.trim() == etag).unwrap_or(true);
    let spec = if honour_range { parse_range(range_header, len) } else { RangeSpec::Full };

    match spec {
        RangeSpec::Full => {
            resp_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            Ok((StatusCode::OK, resp_headers, Body::from_stream(ReaderStream::new(file))).into_response())
        }
        RangeSpec::Partial(start, end) => {
            file.seek(std::io::SeekFrom::Start(start)).await.map_err(anyhow::Error::from)?;
            let n = end - start + 1;
            resp_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(n));
            resp_headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")).map_err(anyhow::Error::from)?,
            );
            let body = Body::from_stream(ReaderStream::new(file.take(n)));
            Ok((StatusCode::PARTIAL_CONTENT, resp_headers, body).into_response())
        }
        RangeSpec::Unsatisfiable => {
            resp_headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{len}")).map_err(anyhow::Error::from)?,
            );
            resp_headers.remove(header::CACHE_CONTROL);
            Ok((StatusCode::RANGE_NOT_SATISFIABLE, resp_headers).into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parsing() {
        use RangeSpec::*;
        assert_eq!(parse_range(None, 100), Full);
        assert_eq!(parse_range(Some("bytes=0-9"), 100), Partial(0, 9));
        assert_eq!(parse_range(Some("bytes=90-"), 100), Partial(90, 99));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Partial(90, 99));
        assert_eq!(parse_range(Some("bytes=-1000"), 100), Partial(0, 99));
        assert_eq!(parse_range(Some("bytes=50-5000"), 100), Partial(50, 99));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=9-3"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 100), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), Full);
        assert_eq!(parse_range(Some("items=0-1"), 100), Full);
        assert_eq!(parse_range(Some("bytes=abc"), 100), Full);
        assert_eq!(parse_range(Some("bytes=0-0"), 0), Unsatisfiable);
    }
}
