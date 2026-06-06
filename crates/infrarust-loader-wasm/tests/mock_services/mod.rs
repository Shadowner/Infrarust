//! Minimal mock implementations of plugin API services, for the fixture tests.
//! Shared across test binaries, so not every item is used by each.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use infrarust_api::error::{PlayerError, ServiceError};
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::{BanEntry, BanTarget};
use infrarust_api::services::config_service::{ConfigService, ServerConfig};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};

pub struct MockPlayerRegistry;

impl infrarust_api::services::player_registry::private::Sealed for MockPlayerRegistry {}

impl PlayerRegistry for MockPlayerRegistry {
    fn get_player(&self, _username: &str) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_uuid(
        &self,
        _uuid: &uuid::Uuid,
    ) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_id(&self, _id: PlayerId) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_players_on_server(
        &self,
        _server: &ServerId,
    ) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn get_all_players(&self) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn online_count(&self) -> usize {
        0
    }
    fn online_count_on(&self, _server: &ServerId) -> usize {
        0
    }
}

pub struct MockBanService;

impl infrarust_api::services::ban_service::private::Sealed for MockBanService {}

impl infrarust_api::services::ban_service::BanService for MockBanService {
    fn ban(
        &self,
        _target: BanTarget,
        _reason: Option<String>,
        _duration: Option<Duration>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }
    fn unban(&self, _target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn is_banned(&self, _target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn get_ban(
        &self,
        _target: &BanTarget,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn get_all_bans(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(vec![]) })
    }
}

pub struct MockConfigService;

impl infrarust_api::services::config_service::private::Sealed for MockConfigService {}

impl ConfigService for MockConfigService {
    fn get_server_config(&self, _server: &ServerId) -> Option<ServerConfig> {
        None
    }
    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        vec![]
    }
    fn get_value(&self, _key: &str) -> Option<String> {
        None
    }
}

/// A player registry that reports a fixed online count (the rest stay empty),
/// for exercising `player-registry.online-count` from a guest.
pub struct CountingPlayerRegistry {
    pub count: usize,
}

impl infrarust_api::services::player_registry::private::Sealed for CountingPlayerRegistry {}

impl PlayerRegistry for CountingPlayerRegistry {
    fn get_player(&self, _username: &str) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_uuid(
        &self,
        _uuid: &uuid::Uuid,
    ) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_player_by_id(&self, _id: PlayerId) -> Option<Arc<dyn infrarust_api::player::Player>> {
        None
    }
    fn get_players_on_server(
        &self,
        _server: &ServerId,
    ) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn get_all_players(&self) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
    }
    fn online_count(&self) -> usize {
        self.count
    }
    fn online_count_on(&self, _server: &ServerId) -> usize {
        self.count
    }
}

/// A player that records every message sent to it, so a test can assert what a
/// plugin replied. `get_player_by_id`/`get_player` hand one out; the rest of the
/// registry stays empty, and `online_count` reports a fixed value.
pub struct RecordingPlayerRegistry {
    pub count: usize,
    pub sent: Arc<Mutex<Vec<String>>>,
}

impl RecordingPlayerRegistry {
    fn player(&self, id: PlayerId) -> Arc<dyn Player> {
        Arc::new(RecordingPlayer {
            id,
            profile: GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "tester".to_string(),
                properties: vec![],
            },
            sent: Arc::clone(&self.sent),
        })
    }
}

impl infrarust_api::services::player_registry::private::Sealed for RecordingPlayerRegistry {}

impl PlayerRegistry for RecordingPlayerRegistry {
    fn get_player(&self, _username: &str) -> Option<Arc<dyn Player>> {
        Some(self.player(PlayerId::new(1)))
    }
    fn get_player_by_uuid(&self, _uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        Some(self.player(PlayerId::new(1)))
    }
    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        Some(self.player(id))
    }
    fn get_players_on_server(&self, _server: &ServerId) -> Vec<Arc<dyn Player>> {
        vec![]
    }
    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
        vec![]
    }
    fn online_count(&self) -> usize {
        self.count
    }
    fn online_count_on(&self, _server: &ServerId) -> usize {
        self.count
    }
}

struct RecordingPlayer {
    id: PlayerId,
    profile: GameProfile,
    sent: Arc<Mutex<Vec<String>>>,
}

impl infrarust_api::player::private::Sealed for RecordingPlayer {}

impl Player for RecordingPlayer {
    fn id(&self) -> PlayerId {
        self.id
    }
    fn profile(&self) -> &GameProfile {
        &self.profile
    }
    fn protocol_version(&self) -> ProtocolVersion {
        ProtocolVersion::MINECRAFT_1_21
    }
    fn remote_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }
    fn current_server(&self) -> Option<ServerId> {
        None
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn is_active(&self) -> bool {
        true
    }
    fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.sent.lock().expect("sent lock").push(message.text);
        Ok(())
    }
    fn send_title(&self, _title: TitleData) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_action_bar(&self, _message: Component) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_packet(&self, _packet: RawPacket) -> Result<(), PlayerError> {
        Ok(())
    }
    fn switch_server(&self, _target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        Box::pin(async { Ok(()) })
    }
    fn is_online_mode(&self) -> bool {
        true
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Player
    }
    fn has_permission(&self, _permission: &str) -> bool {
        false
    }
    fn connected_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

/// A config service backed by a fixed key→value map, for exercising
/// `config-service.get-value` from a guest.
pub struct MapConfigService {
    pub values: HashMap<String, String>,
}

impl infrarust_api::services::config_service::private::Sealed for MapConfigService {}

impl ConfigService for MapConfigService {
    fn get_server_config(&self, _server: &ServerId) -> Option<ServerConfig> {
        None
    }
    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        vec![]
    }
    fn get_value(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
}
