use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use infrarust_api::player::Player;
use infrarust_api::types::{Component, PlayerId, ServerId};

use crate::dto::player::{PlayerCountResponse, PlayerDetailResponse, PlayerResponse};
use crate::dto::requests::{BroadcastRequest, KickRequest, MessageRequest, SendRequest};
use crate::error::ApiError;
use crate::response::{
    ApiResponse, MutationResult, PaginatedResponse, PaginationParams, default_page,
    default_per_page, mutation_ok, ok,
};
use crate::state::ApiState;
use crate::util::proxy_mode_str;

#[derive(Debug, Deserialize)]
pub struct PlayerListQuery {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
    pub server: Option<String>,
    pub mode: Option<String>,
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
    Query(query): Query<PlayerListQuery>,
) -> Result<Json<PaginatedResponse<PlayerResponse>>, ApiError> {
    let mut pagination = PaginationParams {
        page: query.page,
        per_page: query.per_page,
    };
    pagination.normalize();

    let mut players = state.player_registry.get_all_players();

    if let Some(ref server_filter) = query.server {
        players.retain(|p| {
            p.current_server()
                .is_some_and(|s| s.as_str() == server_filter)
        });
    }

    if let Some(ref mode_filter) = query.mode {
        players.retain(|p| {
            if let Some(server_id) = p.current_server()
                && let Some(config) = state.config_service.get_server_config(&server_id)
            {
                return proxy_mode_str(config.proxy_mode) == mode_filter.as_str();
            }
            false
        });
    }

    players.sort_by(|a, b| {
        a.profile()
            .username
            .to_lowercase()
            .cmp(&b.profile().username.to_lowercase())
    });

    let responses: Vec<PlayerResponse> = players
        .iter()
        .map(|p| PlayerResponse::from_player(p.as_ref()))
        .collect();

    Ok(Json(pagination.apply(responses)))
}

pub async fn get(
    State(state): State<Arc<ApiState>>,
    Path(id_or_username): Path<String>,
) -> Result<Json<ApiResponse<PlayerDetailResponse>>, ApiError> {
    let player = find_player(&state, &id_or_username)
        .ok_or_else(|| ApiError::NotFound(format!("Player '{id_or_username}' not found")))?;

    Ok(ok(PlayerDetailResponse::from_player(player.as_ref())))
}

pub async fn count(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<PlayerCountResponse>>, ApiError> {
    let players = state.player_registry.get_all_players();
    let total = players.len();

    let mut by_server = std::collections::HashMap::new();
    let mut by_mode = std::collections::HashMap::new();

    for player in &players {
        if let Some(server_id) = player.current_server() {
            *by_server
                .entry(server_id.as_str().to_string())
                .or_insert(0usize) += 1;

            if let Some(config) = state.config_service.get_server_config(&server_id) {
                *by_mode
                    .entry(proxy_mode_str(config.proxy_mode).to_string())
                    .or_insert(0usize) += 1;
            }
        }
    }

    Ok(ok(PlayerCountResponse {
        total,
        by_server,
        by_mode,
    }))
}

pub async fn kick(
    State(state): State<Arc<ApiState>>,
    Path(username): Path<String>,
    Json(body): Json<KickRequest>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let player = find_player(&state, &username)
        .ok_or_else(|| ApiError::NotFound(format!("Player '{username}' not found")))?;

    let reason = body.reason.as_deref().unwrap_or("Kicked by administrator");

    tracing::info!(
        target: "audit",
        action = "kick",
        player = %username,
        reason = %reason,
        source = "admin_api",
        "Player kicked via Admin API"
    );

    player.disconnect(Component::text(reason)).await;

    Ok(mutation_ok(format!("Player '{username}' has been kicked")))
}

pub async fn send(
    State(state): State<Arc<ApiState>>,
    Path(username): Path<String>,
    Json(body): Json<SendRequest>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let player = find_player(&state, &username)
        .ok_or_else(|| ApiError::NotFound(format!("Player '{username}' not found")))?;

    let target = ServerId::new(&body.server);

    tracing::info!(
        target: "audit",
        action = "send",
        player = %username,
        server = %body.server,
        source = "admin_api",
        "Player sent to server via Admin API"
    );

    player
        .switch_server(target)
        .await
        .map_err(|e| ApiError::Conflict(format!("Failed to send player: {e}")))?;

    Ok(mutation_ok(format!(
        "Player '{username}' sent to '{}'",
        body.server
    )))
}

pub async fn message(
    State(state): State<Arc<ApiState>>,
    Path(username): Path<String>,
    Json(body): Json<MessageRequest>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let player = find_player(&state, &username)
        .ok_or_else(|| ApiError::NotFound(format!("Player '{username}' not found")))?;

    tracing::info!(
        target: "audit",
        action = "message",
        player = %username,
        source = "admin_api",
        "Message sent to player via Admin API"
    );

    player
        .send_message(Component::text(&body.text))
        .map_err(|e| ApiError::Conflict(format!("Failed to send message: {e}")))?;

    Ok(mutation_ok(format!("Message sent to '{username}'")))
}

pub async fn broadcast(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<BroadcastRequest>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let players = state.player_registry.get_all_players();
    let mut sent = 0usize;
    let mut failed = 0usize;

    for player in &players {
        match player.send_message(Component::text(&body.text)) {
            Ok(()) => sent += 1,
            Err(_) => failed += 1,
        }
    }

    tracing::info!(
        target: "audit",
        action = "broadcast",
        sent,
        failed,
        source = "admin_api",
        "Broadcast sent via Admin API"
    );

    Ok(mutation_ok(format!(
        "Broadcast sent to {sent} players ({failed} failed)"
    )))
}

fn find_player(state: &ApiState, id_or_username: &str) -> Option<Arc<dyn Player>> {
    // Try username lookup first (most common)
    if let Some(player) = state.player_registry.get_player(id_or_username) {
        return Some(player);
    }

    // Try numeric ID lookup
    if let Ok(id) = id_or_username.parse::<u64>() {
        return state.player_registry.get_player_by_id(PlayerId::new(id));
    }

    None
}
