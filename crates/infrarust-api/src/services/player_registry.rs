//! Player registry service.

use std::sync::Arc;

use crate::player::Player;
use crate::types::{PlayerId, ServerId};

mod private {
    /// Sealed — only the proxy implements [`PlayerRegistry`](super::PlayerRegistry).
    pub trait Sealed {}
}

/// Registry of all players connected to the proxy.
///
/// Obtained via [`PluginContext::player_registry()`](crate::plugin::PluginContext::player_registry)
/// or as an `Arc<dyn PlayerRegistry>` via
/// [`PluginContext::player_registry_handle()`](crate::plugin::PluginContext::player_registry_handle).
pub trait PlayerRegistry: Send + Sync + private::Sealed {
    /// Finds a player by username (case-insensitive).
    fn get_player(&self, username: &str) -> Option<Arc<dyn Player>>;

    /// Finds a player by their Mojang UUID.
    fn get_player_by_uuid(&self, uuid: &uuid::Uuid) -> Option<Arc<dyn Player>>;

    /// Finds a player by their session ID.
    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>>;

    /// Returns all players currently connected to a specific server.
    fn get_players_on_server(&self, server: &ServerId) -> Vec<Arc<dyn Player>>;

    /// Returns all players connected to the proxy.
    fn get_all_players(&self) -> Vec<Arc<dyn Player>>;

    /// Returns the total number of connected players.
    fn online_count(&self) -> usize;

    /// Returns the number of players on a specific server.
    fn online_count_on(&self, server: &ServerId) -> usize;
}
