use std::sync::Arc;

use axum::extract::State;
use axum::http::{Request, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::ApiError;
use crate::state::ApiState;

pub async fn auth_middleware(
    State(state): State<Arc<ApiState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let header_value = request.headers().get(header::AUTHORIZATION);

    let auth_str = match header_value {
        None => {
            tracing::warn!(target: "audit", action = "auth_failed", reason = "missing_header", "Authentication failed: missing Authorization header");
            return Err(ApiError::Unauthorized("missing Authorization header".into()));
        }
        Some(value) => value.to_str().map_err(|_| {
            ApiError::BadRequest("authorization header contains invalid characters".into())
        })?,
    };

    if let Some(token) = auth_str.strip_prefix("Bearer ") {
        if state.config.verify_api_key(token) {
            Ok(next.run(request).await)
        } else {
            tracing::warn!(target: "audit", action = "auth_failed", reason = "invalid_key", "Authentication failed: invalid API key");
            Err(ApiError::Unauthorized("invalid API key".into()))
        }
    } else {
        tracing::warn!(target: "audit", action = "auth_failed", reason = "invalid_scheme", "Authentication failed: not a Bearer token");
        Err(ApiError::Unauthorized(
            "authorization header must use Bearer scheme".into(),
        ))
    }
}
