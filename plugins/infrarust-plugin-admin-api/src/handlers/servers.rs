use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use infrarust_api::provider::PluginProviderEvent;
use infrarust_api::services::config_service::ServerConfig;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{ServerAddress, ServerId};

use crate::dto::player::PlayerSummary;
use crate::dto::server::{
    CreateServerRequest, HealthCheckResponse, ServerDetailResponse, ServerResponse,
    UpdateServerRequest,
};
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, mutation_ok, ok};
use crate::state::ApiState;
use crate::util::{parse_proxy_mode, proxy_mode_str};

fn server_state_str(state: &ServerState) -> &'static str {
    match state {
        ServerState::Online => "online",
        ServerState::Offline => "offline",
        ServerState::Starting => "starting",
        ServerState::Stopping => "stopping",
        ServerState::Sleeping => "sleeping",
        ServerState::Crashed => "crashed",
        other => {
            tracing::warn!(?other, "Unknown ServerState variant");
            "unknown"
        }
    }
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<ServerResponse>>>, ApiError> {
    let configs = state.config_service.get_all_server_configs();
    let states: HashMap<String, ServerState> = state
        .server_manager
        .get_all_servers()
        .into_iter()
        .map(|(id, st)| (id.as_str().to_string(), st))
        .collect();

    let mut servers: Vec<ServerResponse> = configs
        .iter()
        .map(|config| {
            let id_str = config.id.as_str().to_string();
            let server_state = states.get(&id_str);
            let player_count = state.player_registry.online_count_on(&config.id);

            ServerResponse {
                is_api_managed: state.server_store.contains(&id_str),
                has_server_manager: config.has_server_manager,
                id: id_str,
                addresses: config
                    .addresses
                    .iter()
                    .map(|a| format!("{}:{}", a.host, a.port))
                    .collect(),
                domains: config.domains.clone(),
                proxy_mode: proxy_mode_str(config.proxy_mode).to_string(),
                state: server_state.map(server_state_str).map(String::from),
                player_count,
            }
        })
        .collect();

    servers.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(ok(servers))
}

pub async fn get(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerDetailResponse>>, ApiError> {
    let server_id = ServerId::new(&id);
    let config = state
        .config_service
        .get_server_config(&server_id)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    let server_state = state.server_manager.get_state(&server_id);
    let players = state.player_registry.get_players_on_server(&server_id);

    let response = ServerDetailResponse {
        is_api_managed: state.server_store.contains(&id),
        has_server_manager: config.has_server_manager,
        id,
        addresses: config
            .addresses
            .iter()
            .map(|a| format!("{}:{}", a.host, a.port))
            .collect(),
        domains: config.domains.clone(),
        proxy_mode: proxy_mode_str(config.proxy_mode).to_string(),
        limbo_handlers: config.limbo_handlers.clone(),
        state: server_state
            .as_ref()
            .map(server_state_str)
            .map(String::from),
        player_count: players.len(),
        players: players
            .iter()
            .map(|p| PlayerSummary::from_player(p.as_ref()))
            .collect(),
    };

    Ok(ok(response))
}

pub async fn start(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let server_id = ServerId::new(&id);

    state
        .config_service
        .get_server_config(&server_id)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    tracing::info!(
        target: "audit",
        action = "server_start",
        server = %id,
        source = "admin_api",
        "Server start requested via Admin API"
    );

    state
        .server_manager
        .start(&server_id)
        .await
        .map_err(|e| ApiError::Conflict(format!("Failed to start server: {e}")))?;

    Ok(mutation_ok(format!("Server '{id}' start requested")))
}

pub async fn stop(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let server_id = ServerId::new(&id);

    state
        .config_service
        .get_server_config(&server_id)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    tracing::info!(
        target: "audit",
        action = "server_stop",
        server = %id,
        source = "admin_api",
        "Server stop requested via Admin API"
    );

    state
        .server_manager
        .stop(&server_id)
        .await
        .map_err(|e| ApiError::Conflict(format!("Failed to stop server: {e}")))?;

    Ok(mutation_ok(format!("Server '{id}' stop requested")))
}

fn parse_address(s: &str) -> Result<ServerAddress, ApiError> {
    // Handle IPv6 bracket notation: [::1]:25565
    if let Some(rest) = s.strip_prefix('[') {
        let (host, port_str) = rest.split_once("]:").ok_or_else(|| {
            ApiError::BadRequest(format!("invalid IPv6 address '{s}': expected [host]:port"))
        })?;
        let port: u16 = port_str
            .parse()
            .map_err(|_| ApiError::BadRequest(format!("invalid port in '{s}'")))?;
        return Ok(ServerAddress {
            host: host.to_string(),
            port,
        });
    }

    let (host, port_str) = s.rsplit_once(':').ok_or_else(|| {
        ApiError::BadRequest(format!("invalid address '{s}': expected host:port"))
    })?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| ApiError::BadRequest(format!("invalid port in '{s}'")))?;
    Ok(ServerAddress {
        host: host.to_string(),
        port,
    })
}

const MAX_SERVER_ID_LEN: usize = 64;

fn validate_server_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty() {
        return Err(ApiError::BadRequest("server ID cannot be empty".into()));
    }
    if id.len() > MAX_SERVER_ID_LEN {
        return Err(ApiError::BadRequest(format!(
            "server ID too long ({} chars, max {MAX_SERVER_ID_LEN})",
            id.len()
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::BadRequest(
            "server ID must be alphanumeric with dashes/underscores".into(),
        ));
    }
    Ok(())
}

pub async fn create(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<CreateServerRequest>,
) -> Result<(StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    validate_server_id(&req.id)?;

    if req.domains.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one domain is required".into(),
        ));
    }
    if req.addresses.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one address is required".into(),
        ));
    }

    // Check for duplicates
    let server_id = ServerId::new(&req.id);
    if state.config_service.get_server_config(&server_id).is_some() {
        return Err(ApiError::Conflict(format!(
            "Server '{}' already exists",
            req.id
        )));
    }

    let addresses: Vec<ServerAddress> = req
        .addresses
        .iter()
        .map(|a| parse_address(a))
        .collect::<Result<Vec<_>, _>>()?;

    let proxy_mode = parse_proxy_mode(&req.proxy_mode)?;

    let config = ServerConfig::new(
        server_id,
        None,
        addresses,
        req.domains,
        proxy_mode,
        req.limbo_handlers,
        0,
        None,
        false,
        false,
    );

    state.server_store.insert(config.clone());

    // Emit to the core provider system
    let sender_guard = state.provider_sender.lock().await;
    if let Some(sender) = sender_guard.as_ref() {
        sender.send(PluginProviderEvent::Added(config)).await;
    }

    tracing::info!(
        target: "audit",
        action = "server_create",
        server = %req.id,
        source = "admin_api",
        "Server created via Admin API"
    );

    Ok((
        StatusCode::CREATED,
        ok(MutationResult {
            success: true,
            message: format!("Server '{}' created", req.id),
            details: None,
        }),
    ))
}

pub async fn update(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateServerRequest>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let existing = state
        .server_store
        .get(&id)
        .ok_or_else(|| ApiError::Forbidden("Only API-created servers can be edited".into()))?;

    let addresses = match req.addresses {
        Some(ref addrs) => {
            if addrs.is_empty() {
                return Err(ApiError::BadRequest(
                    "At least one address is required".into(),
                ));
            }
            addrs
                .iter()
                .map(|a| parse_address(a))
                .collect::<Result<Vec<_>, _>>()?
        }
        None => existing.addresses,
    };

    let domains = match req.domains {
        Some(ref d) => {
            if d.is_empty() {
                return Err(ApiError::BadRequest(
                    "At least one domain is required".into(),
                ));
            }
            d.clone()
        }
        None => existing.domains,
    };

    let proxy_mode = match req.proxy_mode {
        Some(ref m) => parse_proxy_mode(m)?,
        None => existing.proxy_mode,
    };

    let limbo_handlers = req.limbo_handlers.unwrap_or(existing.limbo_handlers);

    let config = ServerConfig::new(
        ServerId::new(&id),
        existing.network,
        addresses,
        domains,
        proxy_mode,
        limbo_handlers,
        existing.max_players,
        existing.disconnect_message,
        existing.send_proxy_protocol,
        existing.has_server_manager,
    );

    state.server_store.insert(config.clone());

    let sender_guard = state.provider_sender.lock().await;
    if let Some(sender) = sender_guard.as_ref() {
        sender.send(PluginProviderEvent::Updated(config)).await;
    }

    tracing::info!(
        target: "audit",
        action = "server_update",
        server = %id,
        source = "admin_api",
        "Server updated via Admin API"
    );

    Ok(mutation_ok(format!("Server '{id}' updated")))
}

pub async fn delete(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    state
        .server_store
        .remove(&id)
        .ok_or_else(|| ApiError::Forbidden("Only API-created servers can be deleted".into()))?;

    let server_id = ServerId::new(&id);
    let sender_guard = state.provider_sender.lock().await;
    if let Some(sender) = sender_guard.as_ref() {
        sender.send(PluginProviderEvent::Removed(server_id)).await;
    }

    tracing::info!(
        target: "audit",
        action = "server_delete",
        server = %id,
        source = "admin_api",
        "Server deleted via Admin API"
    );

    Ok(mutation_ok(format!("Server '{id}' deleted")))
}

pub async fn health_check(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<HealthCheckResponse>>, ApiError> {
    let server_id = ServerId::new(&id);
    let config = state
        .config_service
        .get_server_config(&server_id)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    let result = state.health_checker.check(&config).await;
    state.health_cache.set(&id, result.clone());

    Ok(ok(result))
}

pub async fn health_cached(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<HealthCheckResponse>>, ApiError> {
    let server_id = ServerId::new(&id);

    // Verify server exists
    state
        .config_service
        .get_server_config(&server_id)
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    let cached = state
        .health_cache
        .get(&id)
        .ok_or_else(|| ApiError::NotFound(format!("No health check cached for '{id}'")))?;

    Ok(ok(cached))
}
