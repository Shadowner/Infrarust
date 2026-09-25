//! Mocks over the infrarust-api player traits plus a handler test harness.

use std::collections::HashSet;
use std::sync::Arc;

use infrarust_api::limbo::LimboEntryContext;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::test_util::{MockPlayer, MockPlayerRegistry, RecordingLimboSession};
use infrarust_api::types::{Component, GameProfile, PlayerId, ProfileProperty, ServerId};
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
    component.to_plain()
}

pub fn session_text(session: &RecordingLimboSession) -> String {
    session
        .messages()
        .iter()
        .map(flatten)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn player(id: u64, username: &str) -> Arc<MockPlayer> {
    MockPlayer::new(id, username).into_arc()
}

pub fn admin(id: u64, username: &str) -> Arc<MockPlayer> {
    MockPlayer::new(id, username)
        .with_all_permissions()
        .into_arc()
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
    pub registry: Arc<MockPlayerRegistry>,
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
        let registry = Arc::new(MockPlayerRegistry::new());
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
