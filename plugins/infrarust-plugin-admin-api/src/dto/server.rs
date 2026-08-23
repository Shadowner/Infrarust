use infrarust_api::services::load_balancer::BackendStatus;
use serde::Serialize;

use super::player::PlayerSummary;

#[derive(Serialize)]
pub struct ServerResponse {
    pub id: String,
    pub addresses: Vec<String>,
    pub domains: Vec<String>,
    pub proxy_mode: String,
    pub state: Option<String>,
    pub player_count: usize,
    /// Provider type that supplied the config, e.g. `file` or `plugin:admin_api:api`.
    pub source: String,
    /// `true` if this plugin owns the config file and can rewrite it.
    pub editable: bool,
    /// `true` if the server has a server manager (supports start/stop).
    pub has_server_manager: bool,
}

#[derive(Serialize)]
pub struct ServerDetailResponse {
    pub id: String,
    pub addresses: Vec<String>,
    pub domains: Vec<String>,
    pub proxy_mode: String,
    pub limbo_handlers: Vec<String>,
    pub state: Option<String>,
    pub player_count: usize,
    pub players: Vec<PlayerSummary>,
    pub source: String,
    pub editable: bool,
    pub has_server_manager: bool,
}

#[derive(Serialize)]
pub struct BackendStatusResponse {
    pub address: String,
    pub weight: u32,
    pub effective_weight: u32,
    pub state: &'static str,
    pub active_connections: usize,
    pub healthy_since_secs: Option<u64>,
    pub ejections: u32,
    pub last_failure_secs_ago: Option<u64>,
}

impl BackendStatusResponse {
    pub fn from_status(status: &BackendStatus) -> Self {
        Self {
            address: crate::util::format_address(&status.address),
            weight: status.weight,
            effective_weight: status.effective_weight,
            state: status.state.as_str(),
            active_connections: status.active_connections,
            healthy_since_secs: status.healthy_since_secs,
            ejections: status.ejections,
            last_failure_secs_ago: status.last_failure_secs_ago,
        }
    }
}

#[derive(Serialize)]
pub struct ServerBackendsResponse {
    pub strategy: String,
    pub backends: Vec<BackendStatusResponse>,
}

#[derive(Serialize)]
pub struct ProviderResponse {
    pub provider_type: String,
    pub configs_count: usize,
}

#[derive(Serialize)]
pub struct ValidationResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct HealthCheckResponse {
    pub online: bool,
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motd: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub motd_plain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_protocol: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players_online: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players_max: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_sample: Vec<PlayerSampleResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub checked_at: String,
}

impl HealthCheckResponse {
    pub fn error(msg: &str) -> Self {
        Self {
            online: false,
            latency_ms: None,
            motd: None,
            motd_plain: None,
            version_name: None,
            version_protocol: None,
            players_online: None,
            players_max: None,
            player_sample: vec![],
            favicon: None,
            error: Some(msg.to_string()),
            checked_at: crate::util::now_iso8601(),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct PlayerSampleResponse {
    pub name: String,
    pub id: String,
}
