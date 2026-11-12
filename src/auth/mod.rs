//! Who is calling and what they may do.
//!
//! One middleware ([`middleware`]) resolves every `/api/v1` request to an
//! optional [`Principal`]: an owner logged in with a session cookie, or an API
//! key. It also applies the CSRF rules for cookies and the per-key rate
//! limits. Handlers then take the [`Auth`] extractor and call
//! [`Principal::require`] for the scope they need.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::config::Config;
use crate::db::gifs::Visibility;
use crate::db::{keys as key_db, now_ms, sessions};
use crate::error::ApiError;

pub mod accounts;
pub mod keys;
pub mod password;
pub mod ratelimit;

pub const COOKIE: &str = "unigif_session";
const MINUTE_MS: i64 = 60_000;
/// A session is only rewritten this often when it is used.
const SESSION_TOUCH_MS: i64 = 5 * MINUTE_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Upload,
    Tag,
    Delete,
    Admin,
}

impl Scope {
    pub const ALL: [Scope; 5] = [Scope::Read, Scope::Upload, Scope::Tag, Scope::Delete, Scope::Admin];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Upload => "upload",
            Scope::Tag => "tag",
            Scope::Delete => "delete",
            Scope::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|c| c.as_str() == s)
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scopes(u8);

impl Scopes {
    pub fn all() -> Self {
        Scopes(Scope::ALL.iter().fold(0, |a, s| a | s.bit()))
    }

    pub fn from_slice(list: &[Scope]) -> Self {
        Scopes(list.iter().fold(0, |a, s| a | s.bit()))
    }

    /// `"read upload"` (space or comma separated). Unknown names are an error.
    pub fn parse_list(text: &str) -> Result<Self, String> {
        let mut out = Scopes::default();
        for part in text.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()) {
            let s = Scope::parse(part).ok_or_else(|| format!("unknown scope {part:?}"))?;
            out.0 |= s.bit();
        }
        if out.0 == 0 {
            return Err("at least one scope is required".into());
        }
        Ok(out)
    }

    /// A named preset from the key-creation screen. `admin` is never in one.
    pub fn preset(name: &str) -> Option<Self> {
        match name {
            "read_only" => Some(Self::from_slice(&[Scope::Read])),
            "uploader" => Some(Self::from_slice(&[Scope::Upload])),
            "picker" => Some(Self::from_slice(&[Scope::Read, Scope::Upload])),
            "full" => Some(Self::from_slice(&[Scope::Read, Scope::Upload, Scope::Tag, Scope::Delete])),
            _ => None,
        }
    }

    pub fn contains(self, s: Scope) -> bool {
        self.0 & s.bit() != 0
    }

    pub fn names(self) -> Vec<&'static str> {
        Scope::ALL.into_iter().filter(|s| self.contains(*s)).map(Scope::as_str).collect()
    }

    pub fn to_db(self) -> String {
        self.names().join(" ")
    }
}

#[derive(Debug, Clone)]
pub enum Identity {
    Owner { user_id: i64, username: String, session_hash: Vec<u8> },
    Key { id: String, name: String, rate_limit: Option<u32> },
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub identity: Identity,
    pub scopes: Scopes,
    pub deny_tags: Vec<String>,
}

impl Principal {
    pub fn has(&self, s: Scope) -> bool {
        self.scopes.contains(s)
    }

    pub fn require(&self, s: Scope) -> Result<(), ApiError> {
        if self.has(s) {
            Ok(())
        } else {
            Err(ApiError::ScopeRequired(s.as_str()))
        }
    }

    pub fn visibility(&self) -> Visibility {
        Visibility { deny: self.deny_tags.clone() }
    }

    pub fn key_id(&self) -> Option<&str> {
        match &self.identity {
            Identity::Key { id, .. } => Some(id),
            Identity::Owner { .. } => None,
        }
    }

    /// For actions that only make sense for a logged-in person.
    pub fn require_owner(&self) -> Result<(i64, &str, &[u8]), ApiError> {
        match &self.identity {
            Identity::Owner { user_id, username, session_hash } => Ok((*user_id, username, session_hash)),
            Identity::Key { .. } => {
                Err(ApiError::Forbidden("this action needs a logged-in session, not an API key".into()))
            }
        }
    }
}

/// Extractor for handlers: the authenticated caller, or `401`.
pub struct Auth(pub Principal);

impl FromRequestParts<App> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &App) -> Result<Self, ApiError> {
        parts.extensions.get::<Principal>().cloned().map(Auth).ok_or(ApiError::Unauthorized)
    }
}

/// The caller's address (see [`client_ip`]), inserted for every request.
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

/// Rate-limit outcome carried back out to the response headers.
struct RateHeaders(ratelimit::Decision);

pub async fn middleware(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let mut rate = None;
    let ip = client_ip(req.headers(), peer_ip(&req), &app.cfg.auth.trusted_proxies);
    req.extensions_mut().insert(ClientIp(ip));
    let mut resp = match authenticate(&app, &mut req, &mut rate).await {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    };
    if let Some(RateHeaders(d)) = rate {
        let h = resp.headers_mut();
        h.insert("ratelimit-limit", HeaderValue::from(d.limit));
        h.insert("ratelimit-remaining", HeaderValue::from(d.remaining));
        h.insert("ratelimit-reset", HeaderValue::from(d.reset_secs));
    }
    resp
}

async fn authenticate(app: &App, req: &mut Request, rate: &mut Option<RateHeaders>) -> Result<(), ApiError> {
    reject_key_in_query(req)?;
    let unsafe_method = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let now = now_ms();
    // Whether or not the router stripped the nest prefix, compare without it.
    let path = req.uri().path().to_string();
    let path = path.strip_prefix("/api/v1").unwrap_or(&path).to_string();

    let mut via_cookie = false;
    let principal = if let Some(token) = presented_key(req.headers())? {
        Some(key_principal(app, &token, now).await?)
    } else if let Some(p) = session_principal(app, req.headers(), now).await? {
        via_cookie = true;
        Some(p)
    } else {
        None
    };

    if unsafe_method && (via_cookie || path == "/auth/login") {
        check_same_origin(&app.cfg, req.headers())?;
    }
    if via_cookie && unsafe_method {
        let json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim().to_ascii_lowercase().starts_with("application/json"));
        if !json && !req.headers().contains_key("x-unigif-request") {
            return Err(ApiError::Forbidden(
                "cookie-authenticated writes must send JSON or the X-Unigif-Request header".into(),
            ));
        }
    }

    if let Some(p) = principal {
        if let Identity::Key { id, rate_limit, .. } = &p.identity {
            let upload = *req.method() == Method::POST && path == "/gifs";
            let (class, default) = if upload {
                ("upload", app.cfg.rate_limit.upload_per_min)
            } else {
                ("read", app.cfg.rate_limit.read_per_min)
            };
            let d = app.limiter.check(id, class, rate_limit.unwrap_or(default));
            *rate = Some(RateHeaders(d));
            if !d.allowed {
                return Err(ApiError::RateLimited { retry_after: d.retry_after });
            }
        }
        req.extensions_mut().insert(p);
    }
    Ok(())
}

/// The caller of a request outside `/api/v1` (media and share pages in
/// private mode): a key or a session cookie, or `None`. No rate limit or CSRF
/// rule applies, since these are plain GETs.
pub async fn resolve(app: &App, headers: &HeaderMap) -> Result<Option<Principal>, ApiError> {
    let now = now_ms();
    if let Some(token) = presented_key(headers)? {
        return key_principal(app, &token, now).await.map(Some);
    }
    session_principal(app, headers, now).await
}

/// In private media mode, the GIF must be visible to a caller with `read`.
/// Hidden GIFs are a 404, like everywhere else. A no-op when media is public.
pub async fn check_media_access(app: &App, headers: &HeaderMap, gif_id: &str) -> Result<(), ApiError> {
    if app.cfg.media.public {
        return Ok(());
    }
    let p = resolve(app, headers).await?.ok_or(ApiError::Unauthorized)?;
    p.require(Scope::Read)?;
    if p.visibility().hides(&app.db, gif_id).await? {
        return Err(ApiError::NotFound("no such media"));
    }
    Ok(())
}

/// Keys in query strings end up in logs, so they are refused outright.
fn reject_key_in_query(req: &Request) -> Result<(), ApiError> {
    let Some(q) = req.uri().query() else { return Ok(()) };
    let bad =
        serde_urlencoded::from_str::<Vec<(String, String)>>(q).unwrap_or_default().iter().any(|(k, v)| {
            matches!(k.to_ascii_lowercase().as_str(), "api_key" | "apikey" | "access_token" | "token")
                || v.starts_with(keys::PREFIX)
        });
    if bad {
        return Err(ApiError::BadRequest(
            "API keys must be sent in the Authorization header, never in the URL".into(),
        ));
    }
    Ok(())
}

/// The key from `Authorization: Bearer …` or `X-API-Key`. A malformed
/// `Authorization` header is an error rather than being ignored, so a client
/// bug is not silently treated as "anonymous".
fn presented_key(h: &HeaderMap) -> Result<Option<String>, ApiError> {
    if let Some(v) = h.get(header::AUTHORIZATION) {
        let v = v.to_str().map_err(|_| ApiError::Unauthorized)?;
        let (scheme, rest) = v.split_once(' ').ok_or(ApiError::Unauthorized)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(ApiError::Unauthorized);
        }
        return Ok(Some(rest.trim().to_string()));
    }
    Ok(h.get("x-api-key").and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string()))
}

async fn key_principal(app: &App, token: &str, now: i64) -> Result<Principal, ApiError> {
    let (id, secret) = keys::parse(token).ok_or(ApiError::Unauthorized)?;
    let row = key_db::get(&app.db, id).await?.ok_or(ApiError::Unauthorized)?;
    if !keys::secret_matches(secret, &row.secret_hash) || !row.is_usable(now) {
        return Err(ApiError::Unauthorized);
    }
    if row.last_used_at.map_or(true, |t| now - t >= MINUTE_MS) {
        key_db::touch_last_used(&app.db, &row.id, now).await?;
    }
    Ok(Principal {
        scopes: Scopes::parse_list(&row.scopes).unwrap_or_default(),
        deny_tags: row.deny_list(),
        identity: Identity::Key {
            rate_limit: row.rate_limit.map(|r| r.clamp(1, u32::MAX as i64) as u32),
            id: row.id,
            name: row.name,
        },
    })
}

async fn session_principal(app: &App, h: &HeaderMap, now: i64) -> Result<Option<Principal>, ApiError> {
    let Some(value) = cookie_value(h, COOKIE) else { return Ok(None) };
    let hash = hash_token(&value);
    let Some(s) = sessions::find_valid(&app.db, &hash, now).await? else { return Ok(None) };
    if now - s.last_seen_at >= SESSION_TOUCH_MS {
        sessions::touch(&app.db, &hash, now, now + session_ms(&app.cfg)).await?;
    }
    Ok(Some(Principal {
        identity: Identity::Owner { user_id: s.user_id, username: s.username, session_hash: hash.to_vec() },
        scopes: Scopes::all(),
        deny_tags: vec![],
    }))
}

pub fn session_ms(cfg: &Config) -> i64 {
    cfg.auth.session_days as i64 * 24 * 3600 * 1000
}

pub fn hash_token(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

/// A fresh random session token (the cookie value).
pub fn new_session_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn cookie_value(h: &HeaderMap, name: &str) -> Option<String> {
    h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(
        |kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == name).then(|| v.to_string())
        },
    )
}

/// `Set-Cookie` value for the session. `Secure` follows `publicUrl`, so plain
/// http development still works.
pub fn session_cookie(cfg: &Config, value: &str, max_age_secs: i64) -> String {
    let secure = if cfg.public_url.starts_with("https://") { "; Secure" } else { "" };
    format!("{COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_secs}{secure}")
}

/// Browsers attach `Origin` to cross-site writes; it must name this server.
fn check_same_origin(cfg: &Config, h: &HeaderMap) -> Result<(), ApiError> {
    let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return Ok(()) };
    let authority = origin.split_once("://").map(|(_, rest)| rest).unwrap_or(origin);
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok());
    let public = cfg.base_url().split_once("://").map(|(_, rest)| rest);
    if Some(authority) == host || (public.is_some() && Some(authority) == public) {
        Ok(())
    } else {
        Err(ApiError::Forbidden("cross-origin request refused".into()))
    }
}

/// The client's address as a string key. `X-Forwarded-For` is only believed
/// when the TCP peer is one of `auth.trustedProxies`.
pub fn client_ip(req_headers: &HeaderMap, peer: Option<IpAddr>, trusted: &[String]) -> String {
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|t| t.parse::<IpAddr>().is_ok_and(|t| t == *ip));
    let Some(peer) = peer else { return "unknown".into() };
    if !is_trusted(&peer) {
        return peer.to_string();
    }
    let forwarded: Vec<IpAddr> = req_headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    forwarded
        .iter()
        .rev()
        .find(|ip| !is_trusted(ip))
        .or(forwarded.first())
        .copied()
        .unwrap_or(peer)
        .to_string()
}

pub fn peer_ip(req: &Request) -> Option<IpAddr> {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_and_print() {
        let s = Scopes::parse_list("read, upload").unwrap();
        assert!(s.contains(Scope::Read) && s.contains(Scope::Upload) && !s.contains(Scope::Tag));
        assert_eq!(s.to_db(), "read upload");
        assert!(Scopes::parse_list("read bogus").is_err());
        assert!(Scopes::parse_list("").is_err());
        assert!(!Scopes::preset("full").unwrap().contains(Scope::Admin));
        assert!(Scopes::all().contains(Scope::Admin));
        assert_eq!(Scopes::preset("picker").unwrap().names(), vec!["read", "upload"]);
    }

    #[test]
    fn cookies_are_found_among_others() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; unigif_session=abc; b=2"));
        assert_eq!(cookie_value(&h, COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    #[test]
    fn forwarded_for_only_from_trusted_peers() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("9.9.9.9, 203.0.113.7, 10.0.0.1"));
        let trusted = vec!["10.0.0.1".to_string()];
        let ip = |s: &str| Some(s.parse().unwrap());
        // Untrusted peer: the header is a lie.
        assert_eq!(client_ip(&h, ip("198.51.100.1"), &trusted), "198.51.100.1");
        // Trusted peer: rightmost address that is not itself a proxy.
        assert_eq!(client_ip(&h, ip("10.0.0.1"), &trusted), "203.0.113.7");
        assert_eq!(client_ip(&h, None, &trusted), "unknown");
    }

    #[test]
    fn same_origin_rule() {
        let mut cfg = Config::default();
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("gif.example.com"));
        assert!(check_same_origin(&cfg, &h).is_ok()); // no Origin header
        h.insert(header::ORIGIN, HeaderValue::from_static("https://gif.example.com"));
        assert!(check_same_origin(&cfg, &h).is_ok());
        h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example"));
        assert!(check_same_origin(&cfg, &h).is_err());
        cfg.public_url = "https://evil.example".into();
        assert!(check_same_origin(&cfg, &h).is_ok());
    }
}
