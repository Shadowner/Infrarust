use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::dto::server::ProviderResponse;
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, ok};
use crate::state::ApiState;

pub async fn list_providers(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<ProviderResponse>>>, ApiError> {
    let configs_count = state.config_service.get_all_server_configs().len();

    Ok(ok(vec![ProviderResponse {
        provider_type: "file".to_string(),
        configs_count,
    }]))
}

pub async fn reload(
    State(_state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    tracing::info!(
        target: "audit",
        action = "config_reload",
        source = "admin_api",
        "Config reload requested via Admin API"
    );

    Err(ApiError::ServiceUnavailable(
        "Config reload is not yet implemented".into(),
    ))
}
