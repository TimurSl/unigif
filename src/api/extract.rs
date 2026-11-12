//! Extractors that fail with problem+json instead of axum's plain-text rejections.

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let axum::Json(v) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(|e| ApiError::BadRequest(e.body_text()))?;
        Ok(Self(v))
    }
}

pub struct ApiQuery<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        let q = parts.uri.query().unwrap_or("");
        serde_urlencoded::from_str(q)
            .map(Self)
            .map_err(|e| ApiError::BadRequest(format!("invalid query: {e}")))
    }
}

/// `Option<Option<T>>` for PATCH bodies: absent = leave alone, `null` = clear.
pub fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(d).map(Some)
}

/// A list of hashtags given either as a JSON array or as free text.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum TagInput {
    List(Vec<String>),
    Text(String),
}

impl TagInput {
    pub fn normalise(&self) -> Result<Vec<String>, ApiError> {
        match self {
            TagInput::List(l) => crate::tags::normalize_all(l),
            TagInput::Text(t) => crate::tags::parse_list(t),
        }
        .map_err(|e| ApiError::BadRequest(e.to_string()))
    }
}
