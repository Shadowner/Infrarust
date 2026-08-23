use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<crate::server_dir::StoreError> for ApiError {
    fn from(error: crate::server_dir::StoreError) -> Self {
        use crate::server_dir::StoreError;
        match error {
            StoreError::InvalidId(_) | StoreError::Document(_) | StoreError::IdMismatch { .. } => {
                ApiError::BadRequest(error.to_string())
            }
            StoreError::Occupied(_) => ApiError::Conflict(error.to_string()),
            StoreError::Serialize { .. } | StoreError::Io { .. } => {
                tracing::error!(error = %error, "Server directory write failed");
                ApiError::Internal("Failed to persist the server config".into())
            }
        }
    }
}

impl From<crate::drain_store::DrainStoreError> for ApiError {
    fn from(error: crate::drain_store::DrainStoreError) -> Self {
        tracing::error!(error = %error, "Drain store write failed");
        ApiError::Internal("Failed to persist the drain state".into())
    }
}

impl From<infrarust_api::services::config_service::ConfigWriteError> for ApiError {
    fn from(error: infrarust_api::services::config_service::ConfigWriteError) -> Self {
        use infrarust_api::services::config_service::ConfigWriteError;
        match &error {
            ConfigWriteError::PermissionDenied => ApiError::Forbidden(error.to_string()),
            ConfigWriteError::Parse(_) | ConfigWriteError::Validation(_) => {
                ApiError::BadRequest(error.to_string())
            }
            // `ConfigWriteError` is `#[non_exhaustive]`, so a wildcard is
            // mandatory here; an unrecognized failure is a server-side one.
            ConfigWriteError::Io(_) | _ => {
                tracing::error!(error = %error, "Proxy config write failed");
                ApiError::Internal("Failed to write the proxy config".into())
            }
        }
    }
}

/// Every current variant means "the caller named something that is not
/// there", so no match is needed and a future variant cannot be mishandled
/// silently.
impl From<infrarust_api::services::load_balancer::LbError> for ApiError {
    fn from(error: infrarust_api::services::load_balancer::LbError) -> Self {
        ApiError::NotFound(error.to_string())
    }
}

#[derive(Serialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Serialize)]
struct ApiErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ApiError::ServiceUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE")
            }
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let body = ApiErrorBody {
            error: ApiErrorDetail {
                code,
                message: self.to_string(),
            },
        };

        (status, Json(body)).into_response()
    }
}
