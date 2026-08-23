use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;

use infrarust_config::ProxyConfig;

use crate::dto::server::{ProviderResponse, ValidationResponse};
use crate::error::ApiError;
use crate::response::{ApiResponse, MutationResult, RestartAwareResult, ok};
use crate::server_dir;
use crate::state::ApiState;

pub async fn list_providers(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<Vec<ProviderResponse>>>, ApiError> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for source in state.config_service.list_server_sources() {
        *counts.entry(source.provider_type).or_default() += 1;
    }

    Ok(ok(counts
        .into_iter()
        .map(|(provider_type, configs_count)| ProviderResponse {
            provider_type,
            configs_count,
        })
        .collect()))
}

pub async fn reload(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<MutationResult>>, ApiError> {
    tracing::info!(
        target: "audit",
        action = "config_reload",
        source = "admin_api",
        "Config reload requested via Admin API"
    );

    let summary = server_dir::reload(&state.server_dir, &state.provider_sender)
        .await
        .ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "the proxy's config provider is not connected yet, nothing was rescanned".into(),
            )
        })?;
    let details = serde_json::to_value(&summary).ok();

    Ok(ok(MutationResult {
        success: true,
        message: format!(
            "Reloaded API-managed servers: {} added, {} updated, {} removed",
            summary.added, summary.updated, summary.removed
        ),
        details,
    }))
}

/// The config the proxy is running on: the file with its CLI overrides
/// applied, defaults filled in and secrets redacted. The file's own text,
/// which a write edits, is [`get_proxy_raw`].
pub async fn get_proxy(
    State(state): State<Arc<ApiState>>,
) -> Result<Json<ApiResponse<serde_json::Value>>, ApiError> {
    let document = state.config_service.get_effective_proxy_config_document();
    let config: ProxyConfig = toml::from_str(&document)
        .map_err(|e| ApiError::Internal(format!("the running proxy config is unreadable: {e}")))?;

    serde_json::to_value(&config)
        .map(ok)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn get_proxy_raw(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        state.config_service.get_proxy_config_document(),
    )
}

/// Rewrites `infrarust.toml`. Nothing is hot-applied: the proxy keeps running
/// on the configuration it started with.
pub async fn update_proxy_raw(
    State(state): State<Arc<ApiState>>,
    text: String,
) -> Result<Json<ApiResponse<RestartAwareResult>>, ApiError> {
    state.config_service.write_proxy_config_document(&text)?;

    tracing::info!(
        target: "audit",
        action = "config_proxy_write",
        source = "admin_api",
        "Proxy config rewritten via Admin API"
    );

    Ok(ok(RestartAwareResult {
        success: true,
        message: "Proxy config written, restart the proxy to apply it".to_string(),
        requires_restart: true,
    }))
}

/// Checks a global config without persisting it, the same way a write does:
/// where the running proxy's directories point is not this document's business.
/// Accepts TOML when the request carries a text content type, JSON otherwise.
pub async fn validate_proxy(
    headers: HeaderMap,
    body: String,
) -> Json<ApiResponse<ValidationResponse>> {
    let is_toml = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|ct| ct.contains("toml") || ct.starts_with("text/"));

    let parsed: Result<ProxyConfig, String> = if is_toml {
        toml::from_str(&body).map_err(|e| e.to_string())
    } else {
        serde_json::from_str(&body).map_err(|e| e.to_string())
    };

    let errors = match parsed {
        Err(e) => vec![e],
        Ok(config) => infrarust_config::validate_proxy_document(&config)
            .err()
            .map(|e| vec![e.to_string()])
            .unwrap_or_default(),
    };

    ok(ValidationResponse {
        valid: errors.is_empty(),
        errors,
        warnings: vec![],
    })
}
