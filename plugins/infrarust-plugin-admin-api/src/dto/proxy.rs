use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ProxyStatus {
    pub version: String,
    pub uptime_seconds: u64,
    pub uptime_human: String,
    pub players_online: usize,
    pub servers_count: usize,
    pub bind_address: String,
    pub features: Vec<String>,
    pub memory_rss_bytes: Option<u64>,
    pub restart_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_scheduled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_in_seconds: Option<u64>,
}
