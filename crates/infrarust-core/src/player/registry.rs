//! [`PlayerRegistry`] implementation backed by [`ConnectionRegistry`].

use std::sync::Arc;

use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{PlayerId, ServerId};

use crate::registry::ConnectionRegistry;

/// Thin wrapper around [`ConnectionRegistry`] that exposes the API-level
/// [`PlayerRegistry`] trait.
pub struct PlayerRegistryImpl {
    registry: Arc<ConnectionRegistry>,
}

impl PlayerRegistryImpl {
    /// Creates a new registry wrapper.
    pub fn new(registry: Arc<ConnectionRegistry>) -> Self {
        Self { registry }
    }
}

impl infrarust_api::services::player_registry::private::Sealed for PlayerRegistryImpl {}

impl PlayerRegistry for PlayerRegistryImpl {
    fn get_player(&self, username: &str) -> Option<Arc<dyn Player>> {
        self.registry
            .find_by_username(username)
            .map(|s| s as Arc<dyn Player>)
    }

    fn get_player_by_uuid(&self, uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        self.registry
            .find_by_uuid(uuid)
            .map(|s| s as Arc<dyn Player>)
    }

    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        self.registry
            .all()
            .into_iter()
            .find(|s| s.id() == id)
            .map(|s| s as Arc<dyn Player>)
    }

    fn get_players_on_server(&self, server: &ServerId) -> Vec<Arc<dyn Player>> {
        self.registry
            .find_by_server(server.as_str())
            .into_iter()
            .map(|s| s as Arc<dyn Player>)
            .collect()
    }

    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
        self.registry
            .all()
            .into_iter()
            .map(|s| s as Arc<dyn Player>)
            .collect()
    }

    fn online_count(&self) -> usize {
        self.registry.count()
    }

    fn online_count_on(&self, server: &ServerId) -> usize {
        self.registry.count_by_server(server.as_str())
    }
}
