//! Mocks over the infrarust-api player traits plus a handler test harness.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::LimboEntryContext;
use infrarust_api::limbo::test_util::RecordingLimboSession;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProfileProperty, ProtocolVersion, RawPacket, ServerId,
    TitleData,
};
use tempfile::TempDir;

use crate::account::{AuthAccount, DisplayName, PremiumInfo, Username};
use crate::config::AuthConfig;
use crate::handler::AuthHandler;
use crate::password;
use crate::storage::AuthStorage;
use crate::storage::json::JsonFileStorage;

pub fn profile(id: u64, username: &str) -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::from_u128(u128::from(id)),
        username: username.to_string(),
        properties: Vec::new(),
    }
}

/// A profile with a signed `textures` property (`is_mojang_authenticated() == true`).
pub fn premium_profile(id: u64, username: &str) -> GameProfile {
    GameProfile {
        properties: vec![ProfileProperty {
            name: "textures".to_string(),
            value: "data".to_string(),
            signature: Some("signed".to_string()),
        }],
        ..profile(id, username)
    }
}

pub fn limbo_session(id: u64, username: &str) -> Arc<RecordingLimboSession> {
    limbo_session_with(id, profile(id, username))
}

pub fn limbo_session_with(id: u64, profile: GameProfile) -> Arc<RecordingLimboSession> {
    RecordingLimboSession::new(
        PlayerId::new(id),
        profile,
        LimboEntryContext::InitialConnection {
            target_server: ServerId::new("lobby"),
        },
    )
}

pub fn flatten(component: &Component) -> String {
    let mut out = component.text.clone();
    for child in &component.extra {
        out.push_str(&flatten(child));
    }
    out
}

pub fn session_text(session: &RecordingLimboSession) -> String {
    session
        .messages()
        .iter()
        .map(flatten)
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct MockPlayer {
    id: PlayerId,
    profile: GameProfile,
    admin: bool,
    messages: Mutex<Vec<Component>>,
}

impl MockPlayer {
    pub fn new(id: u64, username: &str) -> Arc<Self> {
        Self::build(id, username, false)
    }

    pub fn admin(id: u64, username: &str) -> Arc<Self> {
        Self::build(id, username, true)
    }

    fn build(id: u64, username: &str, admin: bool) -> Arc<Self> {
        Arc::new(Self {
            id: PlayerId::new(id),
            profile: profile(id, username),
            admin,
            messages: Mutex::new(Vec::new()),
        })
    }

    pub fn sent_text(&self) -> String {
        self.messages
            .lock()
            .expect("lock poisoned")
            .iter()
            .map(flatten)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl infrarust_api::player::private::Sealed for MockPlayer {}

impl Player for MockPlayer {
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
        ([127, 0, 0, 1], 25565).into()
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
        self.messages.lock().expect("lock poisoned").push(message);
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
        false
    }

    fn permission_level(&self) -> PermissionLevel {
        if self.admin {
            PermissionLevel::Admin
        } else {
            PermissionLevel::Player
        }
    }

    fn has_permission(&self, _permission: &str) -> bool {
        self.admin
    }

    fn connected_at(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Default)]
pub struct MockRegistry {
    players: Mutex<Vec<Arc<MockPlayer>>>,
}

impl MockRegistry {
    pub fn add(&self, player: Arc<MockPlayer>) {
        self.players.lock().expect("lock poisoned").push(player);
    }

    fn find(&self, pred: impl Fn(&MockPlayer) -> bool) -> Option<Arc<dyn Player>> {
        self.players
            .lock()
            .expect("lock poisoned")
            .iter()
            .find(|p| pred(p))
            .map(|p| Arc::clone(p) as Arc<dyn Player>)
    }
}

impl infrarust_api::services::player_registry::private::Sealed for MockRegistry {}

impl PlayerRegistry for MockRegistry {
    fn get_player(&self, username: &str) -> Option<Arc<dyn Player>> {
        self.find(|p| p.profile.username.eq_ignore_ascii_case(username))
    }

    fn get_player_by_uuid(&self, uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        self.find(|p| p.profile.uuid == *uuid)
    }

    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        self.find(|p| p.id == id)
    }

    fn get_players_on_server(&self, _server: &ServerId) -> Vec<Arc<dyn Player>> {
        Vec::new()
    }

    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
        self.players
            .lock()
            .expect("lock poisoned")
            .iter()
            .map(|p| Arc::clone(p) as Arc<dyn Player>)
            .collect()
    }

    fn online_count(&self) -> usize {
        self.players.lock().expect("lock poisoned").len()
    }

    fn online_count_on(&self, _server: &ServerId) -> usize {
        0
    }
}

pub fn fast_config() -> AuthConfig {
    let mut config = AuthConfig::default();
    config.hashing.argon2_memory_cost = 1024;
    config.hashing.argon2_time_cost = 1;
    config.security.title_reminder_interval_seconds = 0;
    config
}

pub struct TestEnv {
    pub handler: Arc<AuthHandler>,
    pub storage: Arc<dyn AuthStorage>,
    pub registry: Arc<MockRegistry>,
    pub config: Arc<AuthConfig>,
    _dir: TempDir,
}

impl TestEnv {
    pub async fn new() -> Self {
        Self::with_config(fast_config()).await
    }

    pub async fn with_config(config: AuthConfig) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage: Arc<dyn AuthStorage> = Arc::new(
            JsonFileStorage::load_or_create(dir.path(), "accounts.json")
                .await
                .expect("storage"),
        );
        let config = Arc::new(config);
        let registry = Arc::new(MockRegistry::default());
        let dummy_hash = password::generate_dummy_hash(&config.hashing)
            .await
            .expect("dummy hash");
        let handler = Arc::new(AuthHandler::new(
            Arc::clone(&storage),
            Arc::clone(&config),
            Arc::clone(&registry) as Arc<dyn PlayerRegistry>,
            dummy_hash,
            HashSet::new(),
            None,
        ));
        Self {
            handler,
            storage,
            registry,
            config,
            _dir: dir,
        }
    }

    pub async fn create_account(&self, username: &str, password: Option<&str>) {
        let password_hash = match password {
            Some(pw) => Some(
                password::hash_password(pw, &self.config.hashing)
                    .await
                    .expect("hash"),
            ),
            None => None,
        };
        let account = AuthAccount {
            username: Username::new(username),
            display_name: DisplayName::new(username),
            password_hash,
            registered_at: chrono::Utc::now(),
            last_login: None,
            last_ip: None,
            login_count: 0,
            premium_info: None,
        };
        self.storage
            .create_account(&account)
            .await
            .expect("create account");
    }

    pub async fn set_premium_info(&self, username: &str, force_cracked: bool) {
        let info = PremiumInfo {
            mojang_uuid: uuid::Uuid::from_u128(0xfeed),
            force_cracked,
            first_premium_login: chrono::Utc::now(),
            last_premium_login: None,
        };
        self.storage
            .update_premium_info(&Username::new(username), Some(info))
            .await
            .expect("premium info");
    }
}
