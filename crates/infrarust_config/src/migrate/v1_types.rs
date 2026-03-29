//! V1 configuration types for YAML deserialization.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V1ServerConfig {
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub proxy_mode: Option<String>,
    #[serde(default)]
    pub send_proxy_protocol: Option<bool>,
    #[serde(default)]
    pub proxy_protocol_version: Option<u8>,
    #[serde(default)]
    pub backend_domain: Option<String>,
    #[serde(default)]
    pub rewrite_domain: Option<bool>,
    #[serde(default)]
    pub config_id: Option<String>,
    #[serde(default, alias = "server_manager")]
    pub server_manager: Option<V1ServerManager>,
    #[serde(default)]
    pub motds: Option<V1Motds>,
    #[serde(default)]
    pub filters: Option<V1Filters>,
    #[serde(default)]
    pub caches: Option<V1Caches>,
}

#[derive(Debug, Deserialize)]
pub struct V1ServerManager {
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub empty_shutdown_time: Option<u64>,
    #[serde(default)]
    pub local_provider: Option<V1LocalProvider>,
}

#[derive(Debug, Deserialize)]
pub struct V1LocalProvider {
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub startup_string: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct V1Motds {
    #[serde(default)]
    pub online: Option<V1MotdEntry>,
    #[serde(default)]
    pub offline: Option<V1MotdEntry>,
    #[serde(default)]
    pub unreachable: Option<V1MotdEntry>,
    #[serde(default)]
    pub starting: Option<V1MotdEntry>,
    #[serde(default)]
    pub stopping: Option<V1MotdEntry>,
    #[serde(default)]
    pub shutting_down: Option<V1MotdEntry>,
    #[serde(default)]
    pub crashed: Option<V1MotdEntry>,
    #[serde(default)]
    pub unknown: Option<V1MotdEntry>,
    #[serde(default)]
    pub unable_status: Option<V1MotdEntry>,
}

#[derive(Debug, Deserialize)]
pub struct V1MotdEntry {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub version_name: Option<String>,
    #[serde(default)]
    pub max_players: Option<u32>,
    #[serde(default)]
    pub online_players: Option<u32>,
    #[serde(default)]
    pub protocol_version: Option<i32>,
    #[serde(default)]
    pub favicon: Option<String>,
    #[serde(default)]
    pub samples: Vec<V1MotdSample>,
}

#[derive(Debug, Deserialize)]
pub struct V1MotdSample {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V1Filters {
    #[serde(default)]
    pub rate_limiter: Option<V1GenericFilter>,
    #[serde(default, alias = "ip_filter")]
    pub ip_filter: Option<V1IpFilter>,
    #[serde(default, alias = "id_filter")]
    pub id_filter: Option<V1GenericFilter>,
    #[serde(default, alias = "name_filter")]
    pub name_filter: Option<V1GenericFilter>,
    #[serde(default)]
    pub ban: Option<V1GenericFilter>,
}

#[derive(Debug, Deserialize)]
pub struct V1GenericFilter {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct V1IpFilter {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub blacklist: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct V1Caches {
    #[serde(default)]
    pub status_ttl_seconds: Option<u64>,
    #[serde(default)]
    pub max_status_entries: Option<usize>,
}
