use std::collections::HashMap;
use std::time::SystemTime;

use infrarust_api::player::Player;
use serde::Serialize;

use crate::util::{format_duration, format_system_time};

#[derive(Serialize)]
pub struct PlayerResponse {
    pub id: u64,
    pub username: String,
    pub uuid: String,
    pub ip: String,
    pub server: Option<String>,
    pub is_active: bool,
    pub connected_since: String,
    pub connected_duration: String,
}

impl PlayerResponse {
    pub fn from_player(player: &dyn Player) -> Self {
        let connected_at = player.connected_at();
        let duration = SystemTime::now()
            .duration_since(connected_at)
            .unwrap_or_default();

        Self {
            id: player.id().as_u64(),
            username: player.profile().username.clone(),
            uuid: player.profile().uuid.to_string(),
            ip: player.remote_addr().ip().to_string(),
            server: player.current_server().map(|s| s.as_str().to_string()),
            is_active: player.is_active(),
            connected_since: format_system_time(connected_at),
            connected_duration: format_duration(duration),
        }
    }
}

#[derive(Serialize)]
pub struct PlayerDetailResponse {
    #[serde(flatten)]
    pub base: PlayerResponse,
    pub protocol_version: i32,
    pub remote_addr_full: String,
}

impl PlayerDetailResponse {
    pub fn from_player(player: &dyn Player) -> Self {
        Self {
            base: PlayerResponse::from_player(player),
            protocol_version: player.protocol_version().raw(),
            remote_addr_full: player.remote_addr().ip().to_string(),
        }
    }
}

#[derive(Serialize)]
pub struct PlayerCountResponse {
    pub total: usize,
    pub by_server: HashMap<String, usize>,
    pub by_mode: HashMap<String, usize>,
}

#[derive(Serialize)]
pub struct PlayerSummary {
    pub username: String,
    pub uuid: String,
}

impl PlayerSummary {
    pub fn from_player(player: &dyn Player) -> Self {
        Self {
            username: player.profile().username.clone(),
            uuid: player.profile().uuid.to_string(),
        }
    }
}
