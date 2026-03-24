use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::dto::plugin::PluginResponse;
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, ok};
use crate::state::ApiState;

pub async fn list(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<PluginResponse>>>, ApiError> {
    let plugins: Vec<PluginResponse> = state
        .plugin_registry
        .list_plugin_info()
        .into_iter()
        .map(PluginResponse::from_info)
        .collect();

    Ok(ok(plugins))
}

pub async fn get(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<PluginResponse>>, ApiError> {
    let info = state
        .plugin_registry
        .plugin_info(&id)
        .ok_or_else(|| ApiError::NotFound(format!("Plugin '{id}' not found")))?;

    Ok(ok(PluginResponse::from_info(info)))
}

pub async fn disable(
    State(_state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    tracing::info!(
        target: "audit",
        action = "plugin_disable",
        plugin = %id,
        source = "admin_api",
        "Plugin disable requested via Admin API"
    );

    Err(ApiError::ServiceUnavailable(
        "Hot-unloading plugins is not yet supported".into(),
    ))
}

pub async fn enable(
    State(_state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    tracing::info!(
        target: "audit",
        action = "plugin_enable",
        plugin = %id,
        source = "admin_api",
        "Plugin enable requested via Admin API"
    );

    Err(ApiError::ServiceUnavailable(
        "Hot-loading plugins is not yet supported".into(),
    ))
}
