//! Minimal mock implementations of plugin API services for testing.

use std::sync::Arc;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanQuery, BanRequest, BanSource, BanTarget, BanVerdict,
    LoginAttempt, UnbanRequest,
};
use infrarust_api::services::config_service::{ServerConfig, ServerSource};
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
    fn get_players_by_ip(
        &self,
        _ip: std::net::IpAddr,
    ) -> Vec<Arc<dyn infrarust_api::player::Player>> {
        vec![]
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
    fn check<'a>(
        &'a self,
        _attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move {
            Ok(BanEntry::new(
                "1",
                request.target,
                request.source.unwrap_or(BanSource::System),
            ))
        })
    }
    fn unban(
        &self,
        _request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn get<'a>(
        &'a self,
        _target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(async { Ok(BanPage::default()) })
    }
    fn features(&self) -> BanFeatures {
        BanFeatures::new()
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
    fn get_server_document(&self, _server: &ServerId) -> Option<String> {
        None
    }
    fn list_server_sources(&self) -> Vec<ServerSource> {
        vec![]
    }
    fn get_proxy_config_document(&self) -> String {
        String::new()
    }
    fn get_effective_proxy_config_document(&self) -> String {
        String::new()
    }
    fn write_proxy_config_document(
        &self,
        _toml: &str,
    ) -> Result<(), infrarust_api::services::config_service::ConfigWriteError> {
        Err(infrarust_api::services::config_service::ConfigWriteError::PermissionDenied)
    }
    fn get_value(&self, _key: &str) -> Option<String> {
        None
    }
}

pub struct MockLoadBalancerService;

impl infrarust_api::services::load_balancer::private::Sealed for MockLoadBalancerService {}

impl infrarust_api::services::load_balancer::LoadBalancerService for MockLoadBalancerService {
    fn strategy(&self, _server: &ServerId) -> Option<String> {
        None
    }
    fn backends(
        &self,
        _server: &ServerId,
    ) -> Vec<infrarust_api::services::load_balancer::BackendStatus> {
        vec![]
    }
    fn set_drained(
        &self,
        _server: &ServerId,
        _addr: &infrarust_api::types::ServerAddress,
        _drained: bool,
    ) -> Result<(), infrarust_api::services::load_balancer::LbError> {
        Ok(())
    }
    fn reset_backend(
        &self,
        _server: &ServerId,
        _addr: &infrarust_api::types::ServerAddress,
    ) -> Result<(), infrarust_api::services::load_balancer::LbError> {
        Ok(())
    }
}
