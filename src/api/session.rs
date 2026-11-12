//! Owner login, logout, `/me`, password change and session management.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::ApiJson;
use super::types::rfc3339;
use crate::app::App;
use crate::auth::{self, password, Auth, ClientIp, Identity};
use crate::db::{now_ms, sessions, users};
use crate::error::ApiError;

#[derive(Deserialize)]
pub struct LoginBody {
    username: String,
    password: String,
}

/// Run the (slow) hash check off the async threads.
async fn check_password(password: String, phc: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match phc {
        Some(phc) => password::verify(&password, &phc),
        None => {
            password::verify_dummy(&password);
            false
        }
    })
    .await
    .unwrap_or(false)
}

pub async fn login(
    State(app): State<App>,
    headers: HeaderMap,
    Extension(ClientIp(ip)): Extension<ClientIp>,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, ApiError> {
    if let Err(retry_after) = app.login_throttle.check(&ip) {
        return Err(ApiError::RateLimited { retry_after });
    }
    let user = users::find_by_username(&app.db, body.username.trim()).await?;
    let phc = user.as_ref().map(|u| u.password_hash.clone());
    if !check_password(body.password, phc).await {
        app.login_throttle.failure(&ip);
        return Err(ApiError::InvalidLogin);
    }
    let user = user.ok_or(ApiError::InvalidLogin)?;
    app.login_throttle.success(&ip);

    let token = auth::new_session_token();
    let now = now_ms();
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(200).collect::<String>());
    sessions::create(
        &app.db,
        &auth::hash_token(&token),
        user.id,
        ua.as_deref(),
        Some(&ip),
        now,
        now + auth::session_ms(&app.cfg),
    )
    .await?;
    let cookie = auth::session_cookie(&app.cfg, &token, auth::session_ms(&app.cfg) / 1000);
    let mut resp = Json(json!({ "user": { "id": user.id, "username": user.username } })).into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(anyhow::Error::from)?);
    Ok(resp)
}

pub async fn logout(State(app): State<App>, Auth(p): Auth) -> Result<Response, ApiError> {
    let (_, _, hash) = p.require_owner()?;
    sessions::delete(&app.db, hash).await?;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let cookie = auth::session_cookie(&app.cfg, "", 0);
    resp.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).map_err(anyhow::Error::from)?);
    Ok(resp)
}

async fn me_object(app: &App, p: &auth::Principal) -> Result<Value, ApiError> {
    let limits = json!({
        "read_per_min": app.cfg.rate_limit.read_per_min,
        "upload_per_min": app.cfg.rate_limit.upload_per_min,
    });
    Ok(match &p.identity {
        Identity::Owner { user_id, username, .. } => {
            let row = users::find(&app.db, *user_id).await?.ok_or(ApiError::Unauthorized)?;
            let prefs: Value = serde_json::from_str(&row.prefs).unwrap_or_else(|_| json!({}));
            json!({
                "kind": "user", "name": username, "user_id": user_id,
                "scopes": p.scopes.names(), "deny_tags": [], "limits": null, "prefs": prefs,
            })
        }
        Identity::Key { id, name, rate_limit } => {
            let limits = match rate_limit {
                Some(r) => json!({ "read_per_min": r, "upload_per_min": r }),
                None => limits,
            };
            json!({
                "kind": "key", "name": name, "key_id": id,
                "scopes": p.scopes.names(), "deny_tags": p.deny_tags, "limits": limits,
            })
        }
    })
}

pub async fn me(State(app): State<App>, Auth(p): Auth) -> Result<Json<Value>, ApiError> {
    Ok(Json(me_object(&app, &p).await?))
}

#[derive(Deserialize)]
pub struct PatchMe {
    prefs: serde_json::Map<String, Value>,
}

/// Replace the owner's preferences (a small JSON object).
pub async fn patch_me(
    State(app): State<App>,
    Auth(p): Auth,
    ApiJson(body): ApiJson<PatchMe>,
) -> Result<Json<Value>, ApiError> {
    let (user_id, _, _) = p.require_owner()?;
    let text = serde_json::to_string(&body.prefs).map_err(anyhow::Error::from)?;
    if text.len() > 4096 {
        return Err(ApiError::BadRequest("prefs are limited to 4096 bytes".into()));
    }
    users::set_prefs(&app.db, user_id, &text).await?;
    Ok(Json(me_object(&app, &p).await?))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    current: String,
    new: String,
}

pub async fn change_password(
    State(app): State<App>,
    Extension(ClientIp(ip)): Extension<ClientIp>,
    Auth(p): Auth,
    ApiJson(body): ApiJson<PasswordBody>,
) -> Result<StatusCode, ApiError> {
    let (user_id, _, session_hash) = p.require_owner()?;
    if let Err(retry_after) = app.login_throttle.check(&ip) {
        return Err(ApiError::RateLimited { retry_after });
    }
    password::validate_new(&body.new).map_err(ApiError::BadRequest)?;
    let user = users::find(&app.db, user_id).await?.ok_or(ApiError::Unauthorized)?;
    if !check_password(body.current, Some(user.password_hash)).await {
        app.login_throttle.failure(&ip);
        return Err(ApiError::InvalidLogin);
    }
    let new = body.new;
    let phc =
        tokio::task::spawn_blocking(move || password::hash(&new)).await.map_err(anyhow::Error::from)??;
    users::set_password(&app.db, user_id, &phc).await?;
    // Everywhere else is logged out; this session stays.
    sessions::delete_for_user(&app.db, user_id, Some(session_hash)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_sessions(State(app): State<App>, Auth(p): Auth) -> Result<Json<Value>, ApiError> {
    let (user_id, _, current) = p.require_owner()?;
    let rows = sessions::list(&app.db, user_id, now_ms()).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|s| {
            json!({
                "id": s.public_id(), "user_agent": s.user_agent, "ip": s.ip,
                "created_at": rfc3339(s.created_at), "last_seen_at": rfc3339(s.last_seen_at),
                "expires_at": rfc3339(s.expires_at), "current": s.token_hash == current,
            })
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

pub async fn revoke_session(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (user_id, _, _) = p.require_owner()?;
    if sessions::delete_by_public_id(&app.db, user_id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("no such session"))
    }
}
