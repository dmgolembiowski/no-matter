//! Server-wide error type.
//!
//! Every handler returns `AppResult<_>`, where the error variant maps
//! to a status code via `IntoResponse`. The body is a small JSON object
//! `{ "error": "...", "code": "..." }` so the frontend can branch on
//! the symbolic code (`name_taken`, `unauthenticated`, etc.) rather
//! than parsing free-form prose.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not authenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("invalid input: {0}")]
    BadRequest(String),
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
    #[error("database error: {0}")]
    Db(#[from] sea_orm::DbErr),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Conflict(code) => (StatusCode::CONFLICT, code),
            AppError::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, "not_implemented"),
            AppError::Db(_) | AppError::Internal(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.parts();
        // Log the underlying detail server-side for 5xx; only echo a
        // short summary back to the client.
        if status.is_server_error() {
            tracing::error!("{self:#}");
        }
        let body = json!({
            "error": self.to_string(),
            "code": code,
        });
        (status, Json(body)).into_response()
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
