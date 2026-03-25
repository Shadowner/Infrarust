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
}
