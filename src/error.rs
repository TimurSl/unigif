//! API errors rendered as `application/problem+json` with stable `code` strings.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("missing or invalid credentials")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    NotFound(&'static str),
    #[error("upload exceeds the {limit} byte limit")]
    TooLarge { limit: u64 },
    #[error("{0}")]
    UnsupportedMedia(String),
    #[error("{0}")]
    InvalidMedia(String),
    #[error("{0}")]
    NotImplemented(&'static str),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", "Bad request"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "Unauthorized"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden", "Forbidden"),
            Self::NotFound(_) => (StatusCode::NOT_FOUND, "not_found", "Not found"),
            Self::TooLarge { .. } => (StatusCode::PAYLOAD_TOO_LARGE, "too_large", "Payload too large"),
            Self::UnsupportedMedia(_) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media", "Unsupported media type")
            }
            Self::InvalidMedia(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_media", "Invalid media"),
            Self::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, "not_implemented", "Not implemented"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal", "Internal server error"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, title) = self.parts();
        // Internal details go to the log, never to the client.
        let detail = match &self {
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                "an internal error occurred".to_string()
            }
            other => other.to_string(),
        };
        let body = json!({
            "type": format!("about:blank#{code}"),
            "title": title,
            "status": status.as_u16(),
            "detail": detail,
            "code": code,
        });
        let mut resp = (status, body.to_string()).into_response();
        resp.headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/problem+json"));
        resp
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}
