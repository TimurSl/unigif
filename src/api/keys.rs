//! `/keys`: API key management (admin scope).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::extract::{double_option, ApiJson, ApiQuery};
use super::types::rfc3339;
use crate::app::App;
use crate::auth::keys::{self, MintParams};
use crate::auth::{Auth, Scope, Scopes};
use crate::db::keys::{self as key_db, KeyRow};
use crate::db::now_ms;
use crate::error::ApiError;
use crate::tags;

#[derive(Debug, Serialize)]
pub struct KeyObject {
    pub id: String,
    pub name: String,
    pub scopes: Vec<&'static str>,
    pub deny_tags: Vec<String>,
    pub rate_limit: Option<i64>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    /// Only in the response to creating the key, and never again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

impl KeyObject {
    fn from_row(r: &KeyRow, token: Option<String>) -> Self {
        Self {
            scopes: Scopes::parse_list(&r.scopes).unwrap_or_default().names(),
            deny_tags: r.deny_list(),
            id: r.id.clone(),
            name: r.name.clone(),
            rate_limit: r.rate_limit,
            created_at: rfc3339(r.created_at),
            expires_at: r.expires_at.map(rfc3339),
            last_used_at: r.last_used_at.map(rfc3339),
            revoked_at: r.revoked_at.map(rfc3339),
            key: token,
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    include_revoked: bool,
}

pub async fn list(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require(Scope::Admin)?;
    let rows = key_db::list(&app.db, q.include_revoked).await?;
    let items: Vec<KeyObject> = rows.iter().map(|r| KeyObject::from_row(r, None)).collect();
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
    /// `read_only`, `uploader`, `picker` or `full`.
    preset: Option<String>,
    scopes: Option<Vec<String>>,
    #[serde(default)]
    deny_tags: Vec<String>,
    rate_limit: Option<i64>,
    /// RFC 3339.
    expires_at: Option<String>,
    expires_in_days: Option<u32>,
}

fn parse_scopes(preset: Option<&str>, scopes: Option<&[String]>) -> Result<Scopes, ApiError> {
    match (preset, scopes) {
        (Some(_), Some(_)) => Err(ApiError::BadRequest("send either preset or scopes, not both".into())),
        (None, None) => Err(ApiError::BadRequest("send a preset or a list of scopes".into())),
        (Some(p), None) => Scopes::preset(p)
            .ok_or_else(|| ApiError::BadRequest("preset must be read_only, uploader, picker or full".into())),
        (None, Some(s)) => Scopes::parse_list(&s.join(" ")).map_err(ApiError::BadRequest),
    }
}

fn parse_expiry(s: &str) -> Result<i64, ApiError> {
    let t = OffsetDateTime::parse(s, &Rfc3339)
        .map_err(|_| ApiError::BadRequest("expires_at must be an RFC 3339 timestamp".into()))?;
    Ok((t.unix_timestamp_nanos() / 1_000_000) as i64)
}

pub async fn create(
    State(app): State<App>,
    Auth(p): Auth,
    ApiJson(b): ApiJson<CreateBody>,
) -> Result<(StatusCode, Json<KeyObject>), ApiError> {
    p.require(Scope::Admin)?;
    let scopes = parse_scopes(b.preset.as_deref(), b.scopes.as_deref())?;
    let expires_at = match (&b.expires_at, b.expires_in_days) {
        (Some(_), Some(_)) => {
            return Err(ApiError::BadRequest("send either expires_at or expires_in_days".into()))
        }
        (Some(s), None) => Some(parse_expiry(s)?),
        (None, Some(d)) => Some(now_ms() + d as i64 * 86_400_000),
        (None, None) => None,
    };
    if expires_at.is_some_and(|e| e <= now_ms()) {
        return Err(ApiError::BadRequest("expires_at is in the past".into()));
    }
    // Keys are minted by whoever holds the session; a key with admin scope
    // attributes new keys to the owner that created the admin key.
    let created_by = match &p.identity {
        crate::auth::Identity::Owner { user_id, .. } => *user_id,
        crate::auth::Identity::Key { id, .. } => {
            key_db::get(&app.db, id).await?.map(|k| k.created_by).ok_or(ApiError::Unauthorized)?
        }
    };
    let (row, token) = keys::mint(
        &app.db,
        MintParams {
            name: keys::validate_name(&b.name).map_err(ApiError::BadRequest)?,
            scopes,
            deny_tags: tags::normalize_all(&b.deny_tags).map_err(|e| ApiError::BadRequest(e.to_string()))?,
            rate_limit: keys::validate_rate_limit(b.rate_limit).map_err(ApiError::BadRequest)?,
            expires_at,
            created_by,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(KeyObject::from_row(&row, Some(token)))))
}

#[derive(Deserialize)]
pub struct PatchBody {
    name: Option<String>,
    preset: Option<String>,
    scopes: Option<Vec<String>>,
    deny_tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "double_option")]
    rate_limit: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<String>>,
}

pub async fn patch(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<PatchBody>,
) -> Result<Json<KeyObject>, ApiError> {
    p.require(Scope::Admin)?;
    let mut row = key_db::get(&app.db, &id)
        .await?
        .filter(|k| k.revoked_at.is_none())
        .ok_or(ApiError::NotFound("no such key"))?;
    if let Some(n) = &b.name {
        row.name = keys::validate_name(n).map_err(ApiError::BadRequest)?;
    }
    if b.preset.is_some() || b.scopes.is_some() {
        row.scopes = parse_scopes(b.preset.as_deref(), b.scopes.as_deref())?.to_db();
    }
    if let Some(d) = &b.deny_tags {
        let d = tags::normalize_all(d).map_err(|e| ApiError::BadRequest(e.to_string()))?;
        row.deny_tags = serde_json::to_string(&d).map_err(anyhow::Error::from)?;
    }
    if let Some(r) = b.rate_limit {
        row.rate_limit = keys::validate_rate_limit(r).map_err(ApiError::BadRequest)?.map(i64::from);
    }
    if let Some(e) = &b.expires_at {
        row.expires_at = e.as_deref().map(parse_expiry).transpose()?;
    }
    key_db::update(&app.db, &row).await?;
    Ok(Json(KeyObject::from_row(&row, None)))
}

pub async fn revoke(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    p.require(Scope::Admin)?;
    if key_db::revoke(&app.db, &id, now_ms()).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("no such key"))
    }
}
