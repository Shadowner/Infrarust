use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;

use infrarust_api::provider::{PluginProviderEvent, ServerDocument};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::ServerId;
use infrarust_config::ServerConfig;
use infrarust_config::secrets::{SERVER_SECRETS, redact, reinject, still_redacted};
use toml_edit::DocumentMut;

use crate::dto::player::PlayerSummary;
use crate::dto::server::{
    HealthCheckResponse, ServerDetailResponse, ServerResponse, ValidationResponse,
};
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, mutation_ok, ok};
use crate::server_dir::{Committed, DocumentId, MAX_ID_LEN, Ownership, to_document_text};
use crate::state::ApiState;
use crate::util::proxy_mode_str;

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

const UNKNOWN_SOURCE: &str = "unknown";

/// The providers claiming each server id. An id claimed more than once is
/// routed with whichever copy the router iterates first, so the count matters
/// as much as the names.
fn sources(state: &ApiState) -> HashMap<String, Vec<String>> {
    let mut sources: HashMap<String, Vec<String>> = HashMap::new();
    for source in state.config_service.list_server_sources() {
        sources
            .entry(source.id)
            .or_default()
            .push(source.provider_type);
    }
    for providers in sources.values_mut() {
        providers.sort();
    }
    sources
}

fn source_label(providers: Option<&Vec<String>>) -> String {
    providers
        .and_then(|providers| providers.first())
        .cloned()
        .unwrap_or_else(|| UNKNOWN_SOURCE.to_string())
}

/// Whether writing `id` through this plugin reaches the config the proxy
/// actually routes with: it must be served by the document filed under that
/// name, and by nothing else.
fn is_editable(state: &ApiState, id: &str, providers: Option<&Vec<String>>) -> bool {
    state.server_dir.owns(id) && providers.is_none_or(|providers| providers.len() == 1)
}

fn not_ours(id: &str, providers: Option<&Vec<String>>) -> ApiError {
    match providers {
        Some(providers) => ApiError::Forbidden(format!(
            "Server '{id}' comes from the '{}' provider and is read-only here",
            source_label(Some(providers))
        )),
        None => ApiError::NotFound(format!("Server '{id}' not found")),
    }
}

/// Rejects writes to servers this plugin does not own, so a config coming
/// from `servers_dir` or Docker can never be silently shadowed.
fn ensure_editable(state: &ApiState, id: &str) -> Result<DocumentId, ApiError> {
    let sources = sources(state);
    let providers = sources.get(id);

    let Some(document) = DocumentId::new(id) else {
        return Err(not_ours(id, providers));
    };

    match state.server_dir.ownership(&document) {
        Ownership::Owned => {}
        Ownership::Absent => return Err(not_ours(id, providers)),
        Ownership::Shadowed => {
            return Err(ApiError::Conflict(format!(
                "Server '{id}' is served by a document filed under another name; \
                 rename that file to '{id}.toml' before editing it here"
            )));
        }
    }

    if !is_editable(state, id, providers) {
        return Err(ApiError::Conflict(format!(
            "Server '{id}' is also supplied by another provider ({}); \
             remove the duplicate before editing it here",
            providers
                .map(|providers| providers.join(", "))
                .unwrap_or_default()
        )));
    }

    Ok(document)
}

pub async fn list(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<ServerResponse>>>, ApiError> {
    let configs = state.config_service.get_all_server_configs();
    let sources = sources(&state);
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
            let providers = sources.get(&id_str);

            ServerResponse {
                source: source_label(providers),
                editable: is_editable(&state, &id_str, providers),
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
    let sources = sources(&state);
    let providers = sources.get(&id);

    let response = ServerDetailResponse {
        source: source_label(providers),
        editable: is_editable(&state, &id, providers),
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

/// Resolves the identity of a submitted config against the id it is filed
/// under, filling it in when the document leaves it implicit.
fn settle_id(config: &mut ServerConfig, expected: &str) -> Result<DocumentId, ApiError> {
    if config.id.is_none() && config.name.is_none() {
        config.id = Some(expected.to_string());
    }
    let id = config.effective_id();
    if id != expected {
        return Err(ApiError::BadRequest(format!(
            "config identifies as '{id}' but was submitted as '{expected}'"
        )));
    }
    let document = DocumentId::new(expected).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "server ID '{expected}' must be 1 to {MAX_ID_LEN} characters of \
             [a-z0-9._-], must not start with '.' and must not contain '..'"
        ))
    })?;
    infrarust_config::validate_server_config(config)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(document)
}

/// Tells the proxy about a change while it still holds the directory's write
/// lock, so concurrent writers reach the router in the order they reached the
/// disk.
async fn announce(state: &ApiState, committed: Committed<'_>, event: PluginProviderEvent) {
    let sender_guard = state.provider_sender.lock().await;
    if let Some(sender) = sender_guard.as_ref() {
        sender.send(event).await;
    }
    drop(committed);
}

fn document_event(
    committed: &Committed<'_>,
    text: String,
    event: fn(ServerDocument) -> PluginProviderEvent,
) -> PluginProviderEvent {
    event(ServerDocument {
        id: ServerId::new(committed.id().as_str()),
        toml: text,
    })
}

pub async fn create(
    State(state): State<Arc<ApiState>>,
    Json(mut config): Json<ServerConfig>,
) -> Result<(StatusCode, Json<ApiResponse<MutationResult>>), ApiError> {
    if config.id.is_none() && config.name.is_none() {
        return Err(ApiError::BadRequest("server ID is required".into()));
    }

    let id = config.effective_id();
    let document = settle_id(&mut config, &id)?;

    if state.server_dir.ownership(&document) != Ownership::Absent
        || state
            .config_service
            .get_server_config(&ServerId::new(&id))
            .is_some()
    {
        return Err(ApiError::Conflict(format!("Server '{id}' already exists")));
    }

    let text = restore_secrets(&to_document_text(&config)?, None)?;
    let committed = state.server_dir.create(&document, text.clone()).await?;
    let event = document_event(&committed, text, PluginProviderEvent::AddedDocument);
    announce(&state, committed, event).await;

    tracing::info!(
        target: "audit",
        action = "server_create",
        server = %id,
        source = "admin_api",
        "Server created via Admin API"
    );

    Ok((
        StatusCode::CREATED,
        ok(MutationResult {
            success: true,
            message: format!("Server '{id}' created"),
            details: None,
        }),
    ))
}

/// Full replace: the submitted document becomes the server's entire config,
/// every field left out reverts to its default.
pub async fn update(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(mut config): Json<ServerConfig>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let document = ensure_editable(&state, &id)?;
    settle_id(&mut config, &id)?;

    let text = restore_secrets(
        &to_document_text(&config)?,
        state.server_dir.document_text(&id),
    )?;
    let committed = state.server_dir.replace(&document, text.clone()).await?;
    let event = document_event(&committed, text, PluginProviderEvent::UpdatedDocument);
    announce(&state, committed, event).await;

    tracing::info!(
        target: "audit",
        action = "server_update",
        server = %id,
        source = "admin_api",
        "Server replaced via Admin API"
    );

    Ok(mutation_ok(format!("Server '{id}' replaced")))
}

/// The server's document as a client may see it: whatever provider supplies
/// it, with every secret redacted.
fn document_of(state: &ApiState, id: &str) -> Result<String, ApiError> {
    let text = state
        .config_service
        .get_server_document(&ServerId::new(id))
        .or_else(|| state.server_dir.document_text(id))
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|e| ApiError::Internal(format!("the config of '{id}' is unreadable: {e}")))?;
    redact(&mut document, SERVER_SECRETS);
    Ok(document.to_string())
}

/// Puts back the secrets [`document_of`] redacted, so a client can save a
/// document it was never shown the credentials of. `stored` is the document
/// being replaced, if any.
fn restore_secrets(text: &str, stored: Option<String>) -> Result<String, ApiError> {
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    if let Some(stored) = stored.and_then(|text| text.parse::<DocumentMut>().ok()) {
        reinject(&mut document, &stored, SERVER_SECRETS);
    }

    let unrestored = still_redacted(&document, SERVER_SECRETS);
    if !unrestored.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "nothing to restore the redacted {} from: submit the real value",
            unrestored.join(", ")
        )));
    }

    Ok(document.to_string())
}

pub async fn get_raw(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        document_of(&state, &id)?,
    ))
}

/// The whole config as JSON, the read counterpart of the full-replace
/// [`update`]. Unlike [`get`] it loses no field.
pub async fn get_config(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<ServerConfig>>, ApiError> {
    let document = document_of(&state, &id)?;
    let mut config: ServerConfig = toml::from_str(&document)
        .map_err(|e| ApiError::Internal(format!("the config of '{id}' is unreadable: {e}")))?;
    if config.id.is_none() && config.name.is_none() {
        config.id = Some(id);
    }

    Ok(ok(config))
}

/// Full replace from a raw TOML document, stored verbatim.
pub async fn update_raw(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    text: String,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let document = ensure_editable(&state, &id)?;
    let text = restore_secrets(&text, state.server_dir.document_text(&id))?;

    let mut config: ServerConfig =
        toml::from_str(&text).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    settle_id(&mut config, &id)?;

    let committed = state.server_dir.replace(&document, text.clone()).await?;
    let event = document_event(&committed, text, PluginProviderEvent::UpdatedDocument);
    announce(&state, committed, event).await;

    tracing::info!(
        target: "audit",
        action = "server_update_raw",
        server = %id,
        source = "admin_api",
        "Server replaced from raw TOML via Admin API"
    );

    Ok(mutation_ok(format!("Server '{id}' replaced")))
}

/// Checks a config without persisting it. Accepts TOML when the request
/// carries a text content type, JSON otherwise.
pub async fn validate(headers: HeaderMap, body: String) -> Json<ApiResponse<ValidationResponse>> {
    let is_toml = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.contains("toml") || ct.starts_with("text/"));

    let parsed: Result<ServerConfig, String> = if is_toml {
        toml::from_str(&body).map_err(|e| e.to_string())
    } else {
        serde_json::from_str(&body).map_err(|e| e.to_string())
    };

    let response = match parsed {
        Err(e) => ValidationResponse {
            valid: false,
            errors: vec![e],
            warnings: vec![],
        },
        Ok(config) => {
            let errors = infrarust_config::validate_server_config(&config)
                .err()
                .map(|e| vec![e.to_string()])
                .unwrap_or_default();
            ValidationResponse {
                valid: errors.is_empty(),
                errors,
                warnings: infrarust_config::balance_warnings(&config),
            }
        }
    };

    ok(response)
}

pub async fn delete(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    let document = ensure_editable(&state, &id)?;

    let removed = state
        .server_dir
        .remove(&document)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Server '{id}' not found")))?;

    let event = PluginProviderEvent::Removed(ServerId::new(removed.id().as_str()));
    announce(&state, removed, event).await;

    state.drain_store.forget(&id).await?;

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
