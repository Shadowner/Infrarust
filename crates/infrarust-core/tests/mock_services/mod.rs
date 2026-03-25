//! Minimal mock implementations of plugin API services for testing.

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{BanEntry, BanTarget};
use infrarust_api::services::config_service::ServerConfig;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{PlayerId, ServerId};

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

impl infrarust_api::services::config_service::ConfigService for MockConfigService {
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
