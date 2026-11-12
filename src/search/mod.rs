//! Third-party GIF search (KLIPY and GIPHY).
//!
//! The server talks to the providers, so the API key never reaches the
//! browser. Results are normalised to one shape: a small looping preview, and
//! the best file to download into the library (the original GIF, or an MP4
//! when the GIF would exceed the upload limit). Adding a result goes through
//! the ordinary quick add by URL, which brings deduplication, SSRF protection
//! and size limits with it.
//!
//! Tenor is not offered: Google shut its public API down on 30 June 2026.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::app::App;
use crate::db::settings;
use crate::error::ApiError;

pub const MAX_LIMIT: u32 = 50;
pub const DEFAULT_LIMIT: u32 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Klipy,
    Giphy,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Klipy, Provider::Giphy];

    pub fn id(self) -> &'static str {
        match self {
            Provider::Klipy => "klipy",
            Provider::Giphy => "giphy",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Provider::Klipy => "KLIPY",
            Provider::Giphy => "GIPHY",
        }
    }
    /// Where the owner gets a key.
    pub fn keys_url(self) -> &'static str {
        match self {
            Provider::Klipy => "https://klipy.com/developers",
            Provider::Giphy => "https://developers.giphy.com/dashboard/",
        }
    }
    pub fn parse(s: &str) -> Option<Provider> {
        Provider::ALL.into_iter().find(|p| p.id() == s)
    }
    fn setting_key(self) -> String {
        format!("search.{}.api_key", self.id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Entered in Settings.
    Settings,
    /// From the configuration file or environment.
    Config,
}

/// The key to use, and where it came from. A key from Settings wins.
pub async fn api_key(app: &App, p: Provider) -> Result<Option<(String, KeySource)>, sqlx::Error> {
    if let Some(k) = settings::get(&app.db, &p.setting_key()).await?.filter(|k| !k.is_empty()) {
        return Ok(Some((k, KeySource::Settings)));
    }
    let from_cfg = match p {
        Provider::Klipy => &app.cfg.search.klipy_api_key,
        Provider::Giphy => &app.cfg.search.giphy_api_key,
    };
    Ok(from_cfg.clone().filter(|k| !k.trim().is_empty()).map(|k| (k.trim().to_string(), KeySource::Config)))
}

/// Store (or, with `None`, clear) the key entered in Settings.
pub async fn set_api_key(app: &App, p: Provider, key: Option<&str>) -> Result<(), ApiError> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => {
            // Klipy puts the key in the URL path, so only URL-safe characters are accepted.
            if k.len() > 200 || !k.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~".contains(c)) {
                return Err(ApiError::BadRequest(
                    "an API key is at most 200 characters of letters, digits and - _ . ~".into(),
                ));
            }
            settings::set(&app.db, &p.setting_key(), k).await?
        }
        None => settings::delete(&app.db, &p.setting_key()).await?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Normalised results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preview {
    pub url: String,
    /// `video` (a looping MP4 or WebM) or `image`.
    pub kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Download {
    pub url: String,
    /// `gif` or `mp4`.
    pub format: &'static str,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preview: Preview,
    pub download: Download,
    /// The result's page on the provider's site, when it says.
    pub page_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Talking to the providers
// ---------------------------------------------------------------------------

fn client(timeout: u64) -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(timeout.max(1)))
            .connect_timeout(Duration::from_secs(10))
            .user_agent("unigif")
            .build()
            .expect("building the HTTP client")
    })
}

pub struct Query<'a> {
    /// Empty = what is trending.
    pub q: &'a str,
    pub cursor: Option<&'a str>,
    pub limit: u32,
}

pub async fn search(app: &App, p: Provider, key: &str, query: &Query<'_>) -> Result<Page, ApiError> {
    let cfg = &app.cfg.search;
    let limit = query.limit.clamp(1, MAX_LIMIT);
    let cursor: u32 = query.cursor.and_then(|c| c.parse().ok()).unwrap_or(0);
    let trending = query.q.trim().is_empty();
    let max_bytes = app.cfg.upload.max_size.0;
    let name = p.name();

    let request = match p {
        Provider::Giphy => {
            let url = format!(
                "{}/gifs/{}",
                cfg.giphy_base_url.trim_end_matches('/'),
                if trending { "trending" } else { "search" }
            );
            let mut params: Vec<(&str, String)> = vec![
                ("api_key", key.to_string()),
                ("limit", limit.to_string()),
                ("offset", cursor.to_string()),
                ("rating", cfg.rating.clone()),
            ];
            if !trending {
                params.push(("q", query.q.trim().to_string()));
            }
            client(cfg.timeout_secs).get(url).query(&params)
        }
        Provider::Klipy => {
            let url = format!(
                "{}/{}/gifs/{}",
                cfg.klipy_base_url.trim_end_matches('/'),
                key,
                if trending { "trending" } else { "search" }
            );
            let mut params: Vec<(&str, String)> = vec![
                ("page", (cursor.max(1)).to_string()),
                ("per_page", limit.to_string()),
                ("customer_id", "unigif".to_string()),
            ];
            if !trending {
                params.push(("q", query.q.trim().to_string()));
            }
            client(cfg.timeout_secs).get(url).query(&params)
        }
    };

    // `without_url`: the key is part of the URL and must not end up in logs or error messages.
    let resp = request.send().await.map_err(|e| {
        tracing::warn!(provider = name, error = %e.without_url(), "search request failed");
        ApiError::Upstream(format!("could not reach {name}"))
    })?;
    let status = resp.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(ApiError::Upstream(format!("{name} rejected the API key; check it in Settings")));
    }
    if status.as_u16() == 429 {
        return Err(ApiError::Upstream(format!("{name} rate limit reached; try again in a minute")));
    }
    if !status.is_success() {
        return Err(ApiError::Upstream(format!("{name} answered {status}")));
    }
    let bytes = resp.bytes().await.map_err(|e| {
        tracing::warn!(provider = name, error = %e.without_url(), "reading the search response failed");
        ApiError::Upstream(format!("{name} sent a response that could not be read"))
    })?;
    let body: Value = if bytes.len() > 8 << 20 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    let page = match p {
        Provider::Giphy => parse_giphy(&body, max_bytes),
        Provider::Klipy => parse_klipy(&body, max_bytes),
    };
    page.ok_or_else(|| ApiError::Upstream(format!("{name} sent a response in an unexpected shape")))
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// A number that a provider may send as a JSON number or as a string.
fn num(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn url_of(v: &Value) -> Option<String> {
    v.as_str().filter(|s| s.starts_with("https://") || s.starts_with("http://")).map(str::to_string)
}

/// The first candidate that fits `max_bytes`; with none fitting, the smallest known.
fn pick_download(cands: Vec<Download>, max_bytes: u64) -> Option<Download> {
    let fits = |d: &Download| d.bytes.map_or(true, |b| b <= max_bytes);
    if let Some(i) = cands.iter().position(fits) {
        return cands.into_iter().nth(i);
    }
    cands.into_iter().min_by_key(|d| d.bytes.unwrap_or(u64::MAX))
}

/// GIPHY: `{data: [{id, title, url, images: {original: {url, width, height, size, mp4, mp4_size}, fixed_width: {...}}}], pagination: {total_count, count, offset}}`.
/// Sizes and dimensions are strings.
pub fn parse_giphy(body: &Value, max_bytes: u64) -> Option<Page> {
    let data = body.get("data")?.as_array()?;
    let items = data
        .iter()
        .filter_map(|g| {
            let id = text(g.get("id")?)?;
            let img = g.get("images")?;
            let orig = img.get("original")?;
            let mut cands = vec![];
            if let Some(url) = orig.get("url").and_then(url_of) {
                cands.push(Download { url, format: "gif", bytes: orig.get("size").and_then(num) });
            }
            if let Some(url) = orig.get("mp4").and_then(url_of) {
                cands.push(Download { url, format: "mp4", bytes: orig.get("mp4_size").and_then(num) });
            }
            let download = pick_download(cands, max_bytes)?;
            let small = img.get("fixed_width").or_else(|| img.get("fixed_height")).unwrap_or(orig);
            let preview = match small.get("mp4").and_then(url_of) {
                Some(url) => Preview { url, kind: "video" },
                None => Preview {
                    url: small.get("url").and_then(url_of).or_else(|| orig.get("url").and_then(url_of))?,
                    kind: "image",
                },
            };
            Some(Item {
                id,
                title: g.get("title").and_then(text).unwrap_or_default().trim().to_string(),
                width: orig.get("width").and_then(num).map(|n| n as u32),
                height: orig.get("height").and_then(num).map(|n| n as u32),
                preview,
                download,
                page_url: g.get("url").and_then(url_of),
            })
        })
        .collect();
    let p = body.get("pagination");
    let field = |k: &str| p.and_then(|p| p.get(k)).and_then(num);
    let next_cursor = match (field("offset"), field("count"), field("total_count")) {
        (Some(offset), Some(count), Some(total)) if count > 0 && offset + count < total => {
            Some((offset + count).to_string())
        }
        _ => None,
    };
    Some(Page { items, next_cursor })
}

/// KLIPY: `{result, data: {data: [{id, slug, title, file: {hd|md|sm|xs: {gif|webp|jpg|mp4|webm: {url, width, height, size}}}}], current_page, per_page, has_next}}`.
pub fn parse_klipy(body: &Value, max_bytes: u64) -> Option<Page> {
    let outer = body.get("data")?;
    let list = outer.get("data").unwrap_or(outer).as_array()?;
    let items = list
        .iter()
        .filter_map(|g| {
            let id = text(g.get("id")?)?;
            let file = g.get("file").or_else(|| g.get("files"))?;
            let entry =
                |size: &str, fmt: &str| file.get(size)?.get(fmt).filter(|e| url_of(&e["url"]).is_some());
            let mut cands = vec![];
            for fmt in ["gif", "mp4"] {
                for size in ["hd", "md", "sm", "xs"] {
                    if let Some(e) = entry(size, fmt) {
                        cands.push(Download {
                            url: url_of(&e["url"])?,
                            format: if fmt == "gif" { "gif" } else { "mp4" },
                            bytes: e.get("size").and_then(num),
                        });
                        break;
                    }
                }
            }
            let download = pick_download(cands, max_bytes)?;
            let mut preview = None;
            'found: for size in ["sm", "md", "xs", "hd"] {
                for (fmt, kind) in [("mp4", "video"), ("webm", "video"), ("webp", "image"), ("gif", "image")]
                {
                    if let Some(e) = entry(size, fmt) {
                        preview = Some((Preview { url: url_of(&e["url"])?, kind }, e));
                        break 'found;
                    }
                }
            }
            let (preview, shown) = preview?;
            let dims = |e: &Value| (e.get("width").and_then(num), e.get("height").and_then(num));
            let (width, height) = match dims(shown) {
                (Some(w), Some(h)) => (Some(w), Some(h)),
                _ => ["hd", "md", "sm", "xs"]
                    .iter()
                    .find_map(|s| entry(s, "gif").map(dims))
                    .unwrap_or((None, None)),
            };
            Some(Item {
                id,
                title: g.get("title").and_then(text).unwrap_or_default().trim().to_string(),
                width: width.map(|n| n as u32),
                height: height.map(|n| n as u32),
                preview,
                download,
                page_url: g.get("url").and_then(url_of),
            })
        })
        .collect();
    let page = outer.get("current_page").and_then(num).unwrap_or(1);
    let has_next = outer.get("has_next").and_then(Value::as_bool).unwrap_or(false);
    Some(Page { items, next_cursor: has_next.then(|| (page + 1).to_string()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn giphy_results_are_normalised() {
        let body = json!({
            "data": [{
                "id": "abc", "title": " Cat GIF ", "url": "https://giphy.com/gifs/abc",
                "images": {
                    "original": {"url": "https://media.giphy.com/abc.gif", "width": "480", "height": "270",
                                 "size": "900000", "mp4": "https://media.giphy.com/abc.mp4", "mp4_size": "100000"},
                    "fixed_width": {"url": "https://media.giphy.com/abc-200.gif", "mp4": "https://media.giphy.com/abc-200.mp4"}
                }
            }],
            "pagination": {"total_count": 100, "count": 25, "offset": 0}
        });
        let page = parse_giphy(&body, 50 << 20).unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("25"));
        let it = &page.items[0];
        assert_eq!(
            (it.id.as_str(), it.title.as_str(), it.width, it.height),
            ("abc", "Cat GIF", Some(480), Some(270))
        );
        assert_eq!(it.download.format, "gif");
        assert_eq!(it.preview, Preview { url: "https://media.giphy.com/abc-200.mp4".into(), kind: "video" });

        // A GIF over the limit falls back to the MP4; the last page has no cursor.
        let page = parse_giphy(&body, 500_000).unwrap();
        assert_eq!(page.items[0].download.format, "mp4");
        let last = json!({"data": [], "pagination": {"total_count": 25, "count": 25, "offset": 0}});
        assert_eq!(parse_giphy(&last, 1).unwrap().next_cursor, None);
        assert!(parse_giphy(&json!({"nope": 1}), 1).is_none());
    }

    #[test]
    fn klipy_results_are_normalised() {
        let f = |u: &str| json!({"url": format!("https://static.klipy.com/{u}"), "width": 320, "height": 180, "size": 1000});
        let body = json!({
            "result": true,
            "data": {
                "data": [
                    {"id": 77, "slug": "s", "title": "Dance", "file": {
                        "hd": {"gif": f("hd.gif"), "mp4": f("hd.mp4")},
                        "sm": {"gif": f("sm.gif"), "webp": f("sm.webp"), "mp4": f("sm.mp4")}
                    }},
                    {"id": 78, "title": "no files"}
                ],
                "current_page": 2, "per_page": 24, "has_next": true
            }
        });
        let page = parse_klipy(&body, 50 << 20).unwrap();
        assert_eq!(page.items.len(), 1, "an item without files is skipped");
        assert_eq!(page.next_cursor.as_deref(), Some("3"));
        let it = &page.items[0];
        assert_eq!(it.id, "77");
        assert_eq!(
            it.download,
            Download { url: "https://static.klipy.com/hd.gif".into(), format: "gif", bytes: Some(1000) }
        );
        assert_eq!(it.preview, Preview { url: "https://static.klipy.com/sm.mp4".into(), kind: "video" });
        assert_eq!((it.width, it.height), (Some(320), Some(180)));
        let done = json!({"data": {"data": [], "current_page": 1, "has_next": false}});
        assert_eq!(parse_klipy(&done, 1).unwrap().next_cursor, None);
    }

    #[test]
    fn keys_in_debug_output_are_hidden() {
        let cfg = crate::config::SearchConfig { giphy_api_key: Some("sekrit".into()), ..Default::default() };
        assert!(!format!("{cfg:?}").contains("sekrit"));
    }
}
