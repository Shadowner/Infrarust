use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use infrarust_api::services::{ProxyRestart, RestartOptions, RestartRequestError};

use crate::dto::proxy::ProxyStatus;
use crate::dto::requests::RestartRequest;
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
    let scheduled = state
        .restart
        .as_ref()
        .and_then(|restart| restart.scheduled_restart())
        .map(|scheduled| scheduled.at);

    Ok(ok(ProxyStatus {
        version: state.proxy_version.clone(),
        uptime_seconds: uptime.as_secs(),
        uptime_human: util::format_duration(uptime),
        players_online,
        servers_count: servers.len(),
        bind_address: state.config.bind.clone(),
        features: util::get_active_features(),
        memory_rss_bytes: util::get_memory_rss(),
        restart_available: state.restart.is_some(),
        restart_scheduled_at: scheduled.map(util::format_system_time),
        restart_in_seconds: scheduled.map(|at| {
            at.duration_since(SystemTime::now()).map_or(0, |left| {
                left.as_secs() + u64::from(left.subsec_nanos() > 0)
            })
        }),
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
    body: Option<Json<RestartRequest>>,
) -> Result<(StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    let restart = restart_handle(&state)?;
    let RestartRequest {
        delay_seconds,
        message,
    } = body.map(|Json(body)| body).unwrap_or_default();
    let delay = delay_seconds.map(Duration::from_secs);

    tracing::info!(
        target: "audit",
        action = "proxy_restart",
        source = "admin_api",
        delay_seconds,
        announced = message.is_some(),
        "Proxy hot restart requested via Admin API"
    );

    restart
        .request_restart_with(RestartOptions { delay, message })
        .map_err(restart_error)?;

    let message = match delay_seconds.filter(|seconds| *seconds > 0) {
        Some(seconds) => format!("Hot restart scheduled in {seconds} seconds"),
        None => "Hot restart scheduled".to_owned(),
    };
    Ok(accepted(MutationResult {
        success: true,
        message,
        details: None,
    }))
}

pub async fn cancel_restart(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let restart = restart_handle(&state)?;

    tracing::info!(
        target: "audit",
        action = "proxy_restart_cancel",
        source = "admin_api",
        "Proxy hot restart cancellation requested via Admin API"
    );

    restart
        .cancel_restart()
        .map_err(|error| ApiError::Conflict(error.to_string()))?;
    Ok(mutation_ok("Hot restart cancelled"))
}

fn restart_handle(state: &ApiState) -> Result<&Arc<dyn ProxyRestart>, ApiError> {
    state.restart.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("hot restart is not available on this proxy".into())
    })
}

fn restart_error(error: RestartRequestError) -> ApiError {
    match error {
        RestartRequestError::Invalid(_) => ApiError::BadRequest(error.to_string()),
        RestartRequestError::InProgress
        | RestartRequestError::ShuttingDown
        | RestartRequestError::Unsupported(_) => ApiError::Conflict(error.to_string()),
    }
}
