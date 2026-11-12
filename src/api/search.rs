//! `/search`: look things up at KLIPY and GIPHY. Adding a result to the
//! library is the ordinary quick add by URL (`POST /gifs`).

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::extract::{ApiJson, ApiQuery};
use crate::app::App;
use crate::auth::{Auth, Scope};
use crate::error::ApiError;
use crate::search::{self, Page, Provider, Query};

fn provider(id: &str) -> Result<Provider, ApiError> {
    Provider::parse(id).ok_or(ApiError::NotFound("no such search provider"))
}

async fn provider_object(app: &App, p: Provider) -> Result<Value, ApiError> {
    let key = search::api_key(app, p).await?;
    Ok(json!({
        "id": p.id(),
        "name": p.name(),
        "configured": key.is_some(),
        "key_source": key.map(|(_, s)| s),
        "keys_url": p.keys_url(),
    }))
}

pub async fn providers(State(app): State<App>, Auth(p): Auth) -> Result<Json<Value>, ApiError> {
    p.require(Scope::Upload)?;
    let mut items = vec![];
    for prov in Provider::ALL {
        items.push(provider_object(&app, prov).await?);
    }
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
pub struct SearchParams {
    provider: Option<String>,
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

pub async fn find(
    State(app): State<App>,
    Auth(p): Auth,
    ApiQuery(params): ApiQuery<SearchParams>,
) -> Result<Json<Page>, ApiError> {
    p.require(Scope::Upload)?;
    let id = params.provider.as_deref().ok_or_else(|| ApiError::BadRequest("provider is required".into()))?;
    let prov =
        Provider::parse(id).ok_or_else(|| ApiError::BadRequest("provider must be klipy or giphy".into()))?;
    let q = params.q.as_deref().unwrap_or("");
    if q.len() > 200 {
        return Err(ApiError::BadRequest("q is longer than 200 bytes".into()));
    }
    let (key, _) = search::api_key(&app, prov).await?.ok_or_else(|| {
        ApiError::NotConfigured(format!("add a {} API key in Settings to search it", prov.name()))
    })?;
    let query =
        Query { q, cursor: params.cursor.as_deref(), limit: params.limit.unwrap_or(search::DEFAULT_LIMIT) };
    Ok(Json(search::search(&app, prov, &key, &query).await?))
}

#[derive(Deserialize)]
pub struct KeyBody {
    /// `null` or an empty string removes the key saved in Settings.
    key: Option<String>,
}

pub async fn set_key(
    State(app): State<App>,
    Auth(p): Auth,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<KeyBody>,
) -> Result<Json<Value>, ApiError> {
    p.require_owner()?;
    let prov = provider(&id)?;
    search::set_api_key(&app, prov, body.key.as_deref()).await?;
    Ok(Json(provider_object(&app, prov).await?))
}
