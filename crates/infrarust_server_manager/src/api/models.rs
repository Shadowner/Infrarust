use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct StartServerRequest {
    pub server_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StopServerRequest {
    pub server_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u32,
    pub disk_mb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerCommand {
    pub command: String,
    pub timestamp: u64,
    pub status: CommandStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommandStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMetadata {
    pub server_info: ServerInfo,
    pub resources: Option<ServerResourceUsage>,
    pub online_players: Option<u32>,
    pub max_players: Option<u32>,
}
