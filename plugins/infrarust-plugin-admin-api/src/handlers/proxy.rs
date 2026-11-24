use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use infrarust_api::services::RestartRequestError;

use crate::dto::proxy::ProxyStatus;
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, accepted, mutation_ok, ok};
use crate::state::ApiState;
use crate::util;

pub async fn status(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<ProxyStatus>>, ApiError> {
    let uptime = state.start_time.elapsed();
    let players_online = state.player_registry.online_count();
    let servers = state.config_service.get_all_server_configs();

    Ok(ok(ProxyStatus {
        version: state.proxy_version.clone(),
        uptime_seconds: uptime.as_secs(),
        uptime_human: util::format_duration(uptime),
        players_online,
        servers_count: servers.len(),
        bind_address: state.config.bind.clone(),
        features: util::get_active_features(),
        memory_rss_bytes: util::get_memory_rss(),
    }))
}

pub async fn shutdown(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    tracing::info!(
        target: "audit",
        action = "proxy_shutdown",
        source = "admin_api",
        "Proxy shutdown requested via Admin API"
    );

    state.proxy_shutdown.cancel();

    Ok(mutation_ok("Proxy shutdown initiated"))
}

pub async fn restart(
    State(state): State<Arc<ApiState>>,
) -> Result<(StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    let restart = state.restart.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("hot restart is not available on this proxy".into())
    })?;

    tracing::info!(
        target: "audit",
        action = "proxy_restart",
        source = "admin_api",
        "Proxy hot restart requested via Admin API"
    );

    restart.request_restart().map_err(restart_error)?;

    Ok(accepted(MutationResult {
        success: true,
        message: "Hot restart scheduled".to_owned(),
        details: None,
    }))
}

fn restart_error(error: RestartRequestError) -> ApiError {
    match error {
        RestartRequestError::InProgress
        | RestartRequestError::ShuttingDown
        | RestartRequestError::Unsupported(_) => ApiError::Conflict(error.to_string()),
    }
}
