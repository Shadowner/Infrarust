use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use infrarust_api::types::ServerId;

use crate::dto::server::{BackendStatusResponse, ServerBackendsResponse};
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, mutation_ok, ok};
use crate::state::ApiState;
use crate::util::{format_address, parse_address};

fn server_backends(state: &ApiState, id: &str) -> Option<ServerBackendsResponse> {
    let server = ServerId::new(id);
    let strategy = state.load_balancer.strategy(&server)?;
    Some(ServerBackendsResponse {
        strategy,
        backends: state
            .load_balancer
            .backends(&server)
            .iter()
            .map(BackendStatusResponse::from_status)
            .collect(),
    })
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerBackendsResponse>>, ApiError> {
    server_backends(&state, &id)
        .map(ok)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))
}

/// Proxy-wide view, keyed by server id.
pub async fn list_all(
    State(state): State<Arc<ApiState>>,
) -> Json<ApiResponse<BTreeMap<String, ServerBackendsResponse>>> {
    ok(state
        .config_service
        .get_all_server_configs()
        .into_iter()
        .filter_map(|config| {
            let id = config.id.as_str().to_string();
            server_backends(&state, &id).map(|backends| (id, backends))
        })
        .collect())
}

async fn set_drained(
    state: &ApiState,
    id: &str,
    address: &str,
    drained: bool,
) -> Result<(), ApiError> {
    let parsed = parse_address(address)?;
    state
        .load_balancer
        .set_drained(&ServerId::new(id), &parsed, drained)?;

    tracing::info!(
        target: "audit",
        action = if drained { "backend_drain" } else { "backend_enable" },
        server = %id,
        backend = %format_address(&parsed),
        source = "admin_api",
        "Backend drain state changed via Admin API"
    );

    state.drain_store.set(id, &parsed, drained).await?;
    Ok(())
}

pub async fn drain(
    State(state): State<Arc<ApiState>>,
    Path((id, address)): Path<(String, String)>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    set_drained(&state, &id, &address, true).await?;
    Ok(mutation_ok(format!("Backend '{address}' draining")))
}

pub async fn enable(
    State(state): State<Arc<ApiState>>,
    Path((id, address)): Path<(String, String)>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    set_drained(&state, &id, &address, false).await?;
    Ok(mutation_ok(format!("Backend '{address}' back in rotation")))
}

pub async fn reset(
    State(state): State<Arc<ApiState>>,
    Path((id, address)): Path<(String, String)>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let parsed = parse_address(&address)?;
    state
        .load_balancer
        .reset_backend(&ServerId::new(&id), &parsed)?;

    tracing::info!(
        target: "audit",
        action = "backend_reset",
        server = %id,
        backend = %address,
        source = "admin_api",
        "Backend health history cleared via Admin API"
    );

    Ok(mutation_ok(format!("Backend '{address}' reset")))
}
