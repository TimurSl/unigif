//! INTERIM authentication: one static bearer token from the config.
//! It exists so the milestone-1 endpoints are never open to the world; it is
//! replaced by sessions and scoped API keys in milestone 2.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use subtle::ConstantTimeEq;

use crate::app::App;
use crate::error::ApiError;

pub struct Authed;

impl FromRequestParts<App> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &App) -> Result<Self, Self::Rejection> {
        let Some(expected) = app.cfg.auth.bootstrap_token.as_deref().filter(|t| !t.is_empty()) else {
            return Err(ApiError::Forbidden(
                "no API credentials are configured; set auth.bootstrapToken (UNIGIF_AUTH__BOOTSTRAP_TOKEN)",
            ));
        };
        let presented = bearer(parts).ok_or(ApiError::Unauthorized)?;
        if presented.as_bytes().ct_eq(expected.as_bytes()).into() {
            Ok(Authed)
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

fn bearer(parts: &Parts) -> Option<String> {
    let h = &parts.headers;
    if let Some(v) = h.get("authorization").and_then(|v| v.to_str().ok()) {
        let (scheme, rest) = v.split_once(' ')?;
        return scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim().to_string());
    }
    h.get("x-api-key").and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string())
}
