use serde::{Deserialize, Serialize};

use super::player::PlayerSummary;

#[derive(Serialize)]
pub struct ServerResponse {
    pub id: String,
    pub addresses: Vec<String>,
    pub domains: Vec<String>,
    pub proxy_mode: String,
    pub state: Option<String>,
    pub player_count: usize,
    /// `true` if created via the Admin API (editable/deletable).
    pub is_api_managed: bool,
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
    pub is_api_managed: bool,
    pub has_server_manager: bool,
}

#[derive(Serialize)]
pub struct ProviderResponse {
    pub provider_type: String,
    pub configs_count: usize,
}

#[derive(Deserialize)]
pub struct CreateServerRequest {
    pub id: String,
    pub domains: Vec<String>,
    /// Addresses in `"host:port"` format.
    pub addresses: Vec<String>,
    #[serde(default = "default_proxy_mode")]
    pub proxy_mode: String,
    #[serde(default)]
    pub limbo_handlers: Vec<String>,
}

fn default_proxy_mode() -> String {
    "passthrough".to_string()
}

#[derive(Deserialize)]
pub struct UpdateServerRequest {
    pub domains: Option<Vec<String>>,
    pub addresses: Option<Vec<String>>,
    pub proxy_mode: Option<String>,
    pub limbo_handlers: Option<Vec<String>>,
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
