//! Thin typed accessors over the host service imports. Each is a zero-sized
//! handle: use `ctx.player_registry()` or the value directly (e.g. inside a
//! command handler, `Players.online_count()`).

use crate::bindings::{ban_service, config_service, player_registry, server_manager};

pub use crate::bindings::player_registry::Player;
pub use crate::bindings::types::{
    BanEntry, BanTarget, PermissionLevel, PlayerError, ServerConfig, ServerState, ServiceError,
};

pub struct Players;

impl Players {
    #[must_use]
    pub fn online_count(&self) -> u32 {
        player_registry::online_count()
    }
    #[must_use]
    pub fn online_count_on(&self, server: &str) -> u32 {
        player_registry::online_count_on(server)
    }
    #[must_use]
    pub fn get_by_id(&self, id: u64) -> Option<Player> {
        player_registry::get_player_by_id(id)
    }
    #[must_use]
    pub fn get_by_name(&self, username: &str) -> Option<Player> {
        player_registry::get_player(username)
    }
    #[must_use]
    pub fn get_by_uuid(&self, uuid: &str) -> Option<Player> {
        player_registry::get_player_by_uuid(uuid)
    }
    #[must_use]
    pub fn on_server(&self, server: &str) -> Vec<Player> {
        player_registry::get_players_on_server(server)
    }
    #[must_use]
    pub fn all(&self) -> Vec<Player> {
        player_registry::get_all_players()
    }
}

pub struct Servers;

impl Servers {
    #[must_use]
    pub fn state(&self, server: &str) -> Option<ServerState> {
        server_manager::get_state(server)
    }
    pub fn start(&self, server: &str) -> Result<(), ServiceError> {
        server_manager::start(server)
    }
    pub fn stop(&self, server: &str) -> Result<(), ServiceError> {
        server_manager::stop(server)
    }
    #[must_use]
    pub fn all(&self) -> Vec<(String, ServerState)> {
        server_manager::get_all_servers()
    }
}

pub struct Bans;

impl Bans {
    pub fn ban(
        &self,
        target: &BanTarget,
        reason: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(), ServiceError> {
        ban_service::ban(target, reason, duration_ms)
    }
    pub fn unban(&self, target: &BanTarget) -> Result<bool, ServiceError> {
        ban_service::unban(target)
    }
    pub fn is_banned(&self, target: &BanTarget) -> Result<bool, ServiceError> {
        ban_service::is_banned(target)
    }
    pub fn get(&self, target: &BanTarget) -> Result<Option<BanEntry>, ServiceError> {
        ban_service::get_ban(target)
    }
    pub fn all(&self) -> Result<Vec<BanEntry>, ServiceError> {
        ban_service::get_all_bans()
    }
}

pub struct Config;

impl Config {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        config_service::get_value(key)
    }
    #[must_use]
    pub fn server(&self, server: &str) -> Option<ServerConfig> {
        config_service::get_server_config(server)
    }
    #[must_use]
    pub fn servers(&self) -> Vec<ServerConfig> {
        config_service::get_all_server_configs()
    }
}
