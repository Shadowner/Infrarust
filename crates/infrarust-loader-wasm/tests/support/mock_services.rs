#![allow(dead_code)]

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use infrarust_api::error::{PlayerError, ServiceError};
use infrarust_api::event::BoxFuture;
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanQuery, BanRequest, BanSource, BanTarget, BanVerdict,
    LoginAttempt, UnbanRequest,
};
use infrarust_api::services::config_service::{ConfigService, ServerConfig, ServerSource};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};

pub struct MockPlayerRegistry;

impl infrarust_api::services::player_registry::private::Sealed for MockPlayerRegistry {}

impl PlayerRegistry for MockPlayerRegistry {
    fn get_player(&self, _username: &str) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_player_by_uuid(&self, _uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_player_by_id(&self, _id: PlayerId) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_players_by_ip(&self, _ip: IpAddr) -> Vec<Arc<dyn Player>> {
        vec![]
    }
    fn get_players_on_server(&self, _server: &ServerId) -> Vec<Arc<dyn Player>> {
        vec![]
    }
    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
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

pub struct Gate {
    entered: tokio::sync::Notify,
    open: tokio::sync::watch::Sender<bool>,
}

impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            open: tokio::sync::watch::channel(false).0,
        })
    }

    pub async fn entered(&self) {
        self.entered.notified().await;
    }

    pub fn open(&self) {
        self.open.send_replace(true);
    }

    async fn pass(&self) {
        self.entered.notify_one();
        let mut open = self.open.subscribe();
        let _ = open.wait_for(|open| *open).await;
    }
}

pub struct GatedBanService {
    pub gate: Arc<Gate>,
}

impl infrarust_api::services::ban_service::private::Sealed for GatedBanService {}

impl infrarust_api::services::ban_service::BanService for GatedBanService {
    fn check<'a>(
        &'a self,
        _attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(async move {
            self.gate.pass().await;
            Ok(None)
        })
    }
    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move {
            self.gate.pass().await;
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
        Box::pin(async move {
            self.gate.pass().await;
            Ok(None)
        })
    }
    fn get<'a>(
        &'a self,
        _target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async move {
            self.gate.pass().await;
            Ok(None)
        })
    }
    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(async move {
            self.gate.pass().await;
            Ok(BanPage::default())
        })
    }
    fn features(&self) -> BanFeatures {
        BanFeatures::new()
    }
}

pub struct PanickingBanService;

impl infrarust_api::services::ban_service::private::Sealed for PanickingBanService {}

impl infrarust_api::services::ban_service::BanService for PanickingBanService {
    fn check<'a>(
        &'a self,
        _attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        Box::pin(async { panic!("ban service panicked on purpose") })
    }
    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        Box::pin(async move {
            let _ = request;
            panic!("ban service panicked on purpose")
        })
    }
    fn unban(
        &self,
        _request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { panic!("ban service panicked on purpose") })
    }
    fn get<'a>(
        &'a self,
        _target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        Box::pin(async { panic!("ban service panicked on purpose") })
    }
    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        Box::pin(async { panic!("ban service panicked on purpose") })
    }
    fn features(&self) -> BanFeatures {
        BanFeatures::new()
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

pub struct CountingPlayerRegistry {
    pub count: usize,
}

impl infrarust_api::services::player_registry::private::Sealed for CountingPlayerRegistry {}

impl PlayerRegistry for CountingPlayerRegistry {
    fn get_player(&self, _username: &str) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_player_by_uuid(&self, _uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_player_by_id(&self, _id: PlayerId) -> Option<Arc<dyn Player>> {
        None
    }
    fn get_players_by_ip(&self, _ip: IpAddr) -> Vec<Arc<dyn Player>> {
        vec![]
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
    fn get_players_by_ip(&self, _ip: IpAddr) -> Vec<Arc<dyn Player>> {
        vec![]
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
        self.sent
            .lock()
            .expect("sent lock")
            .push(message.as_text().unwrap_or_default().to_owned());
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
    fn has_permission(&self, _permission: &str) -> bool {
        false
    }
    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn connected_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

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
    fn get_value(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
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
