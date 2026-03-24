use std::collections::HashMap;

use serde::Serialize;

#[derive(Serialize)]
pub struct StatsResponse {
    pub players_online: usize,
    pub servers_total: usize,
    pub servers_online: usize,
    pub servers_sleeping: usize,
    pub servers_offline: usize,
    pub bans_active: usize,
    pub uptime_seconds: u64,
    pub memory_rss_bytes: Option<u64>,
    pub players_by_server: HashMap<String, usize>,
    pub servers_by_state: HashMap<String, usize>,
}
