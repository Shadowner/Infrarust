use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use infrarust_api::error::ServiceError;
use infrarust_api::services::server_manager::ServerState;

use crate::dto::stats::StatsResponse;
use crate::error::ApiError;
use crate::response::{ApiResponse, ok};
use crate::state::ApiState;
use crate::util::{active_ban_count, get_memory_rss, server_state_str};

pub async fn overview(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<StatsResponse>>, ApiError> {
    let players = state.player_registry.get_all_players();
    let all_servers = state.server_manager.get_all_servers();
    let bans_active = match active_ban_count(&*state.ban_service).await {
        Ok(count) => count,
        Err(ServiceError::Unavailable(_)) => 0,
        Err(e) => return Err(ApiError::Internal(format!("Failed to fetch bans: {e}"))),
    };

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
        match server_state {
            ServerState::Online => servers_online += 1,
            ServerState::Sleeping => servers_sleeping += 1,
            ServerState::Offline => servers_offline += 1,
            _ => {}
        }
        *servers_by_state
            .entry(server_state_str(server_state).to_string())
            .or_insert(0) += 1;
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
