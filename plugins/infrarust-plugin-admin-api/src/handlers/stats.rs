use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::dto::stats::StatsResponse;
use crate::error::ApiError;
use crate::response::{ApiResponse, ok};
use crate::state::ApiState;
use crate::util::get_memory_rss;

pub async fn overview(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<StatsResponse>>, ApiError> {
    let players = state.player_registry.get_all_players();
    let all_servers = state.server_manager.get_all_servers();
    let bans = state
        .ban_service
        .get_all_bans()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to fetch bans: {e}")))?;

    let bans_active = bans.iter().filter(|b| !b.is_expired()).count();

    let mut players_by_server: HashMap<String, usize> = HashMap::new();
    for player in &players {
        if let Some(server_id) = player.current_server() {
            *players_by_server
                .entry(server_id.as_str().to_string())
                .or_insert(0) += 1;
        }
    }

    let mut servers_by_state: HashMap<String, usize> = HashMap::new();
    let mut servers_online = 0usize;
    let mut servers_sleeping = 0usize;
    let mut servers_offline = 0usize;

    for (_, server_state) in &all_servers {
        let key = match server_state {
            infrarust_api::services::server_manager::ServerState::Online => {
                servers_online += 1;
                "online"
            }
            infrarust_api::services::server_manager::ServerState::Sleeping => {
                servers_sleeping += 1;
                "sleeping"
            }
            infrarust_api::services::server_manager::ServerState::Offline => {
                servers_offline += 1;
                "offline"
            }
            infrarust_api::services::server_manager::ServerState::Starting => "starting",
            infrarust_api::services::server_manager::ServerState::Stopping => "stopping",
            infrarust_api::services::server_manager::ServerState::Crashed => "crashed",
            other => {
                tracing::warn!(?other, "Unknown ServerState variant");
                "unknown"
            }
        };
        *servers_by_state.entry(key.to_string()).or_insert(0) += 1;
    }

    let uptime = state.start_time.elapsed();

    // servers_total counts all configured servers (including those without a manager).
    // servers_online/sleeping/offline only count manager-tracked servers.
    Ok(ok(StatsResponse {
        players_online: players.len(),
        servers_total: state.config_service.get_all_server_configs().len(),
        servers_online,
        servers_sleeping,
        servers_offline,
        bans_active,
        uptime_seconds: uptime.as_secs(),
        memory_rss_bytes: get_memory_rss(),
        players_by_server,
        servers_by_state,
    }))
}
