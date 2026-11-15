//! Auth limbo handler — the core authentication flow.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler, LimboOutcome, SessionEndReason};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, PlayerId, TitleData};
use tokio_util::sync::CancellationToken;

use crate::account::{AuthAccount, DisplayName, PremiumInfo, Username};
use crate::config::AuthConfig;
use crate::password;
use crate::premium::PremiumCache;
use crate::storage::AuthStorage;
use crate::util::parse_colored;

struct AuthSessionState {
    needs_register: bool,
    username: Username,
    force_completed: bool,
}

#[derive(Default)]
struct ConnectionAuth {
    authenticated: bool,
    failed_attempts: u32,
}

pub(crate) enum Attempt {
    Retry { attempts_left: u32 },
    Exhausted,
}

pub struct AuthHandler {
    sessions: DashMap<PlayerId, AuthSessionState>,
    connections: DashMap<PlayerId, ConnectionAuth>,
    storage: Arc<dyn AuthStorage>,
    config: Arc<AuthConfig>,
    player_registry: Arc<dyn PlayerRegistry>,
    dummy_hash: crate::account::PasswordHash,
    blocked_passwords: HashSet<String>,
    premium_cache: Option<Arc<PremiumCache>>,
}

impl AuthHandler {
    pub fn new(
        storage: Arc<dyn AuthStorage>,
        config: Arc<AuthConfig>,
        player_registry: Arc<dyn PlayerRegistry>,
        dummy_hash: crate::account::PasswordHash,
        blocked_passwords: HashSet<String>,
        premium_cache: Option<Arc<PremiumCache>>,
    ) -> Self {
        Self {
            sessions: DashMap::new(),
            connections: DashMap::new(),
            storage,
            config,
            player_registry,
            dummy_hash,
            blocked_passwords,
            premium_cache,
        }
    }

    pub(crate) fn premium_cache(&self) -> Option<&Arc<PremiumCache>> {
        self.premium_cache.as_ref()
    }

    pub(crate) fn storage(&self) -> &Arc<dyn AuthStorage> {
        &self.storage
    }

    pub(crate) fn config(&self) -> &Arc<AuthConfig> {
        &self.config
    }

    pub(crate) fn player_registry(&self) -> &dyn PlayerRegistry {
        self.player_registry.as_ref()
    }

    pub(crate) fn force_complete_session(&self, player_id: PlayerId) -> bool {
        if let Some(mut entry) = self.sessions.get_mut(&player_id) {
            entry.force_completed = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn dummy_hash(&self) -> &crate::account::PasswordHash {
        &self.dummy_hash
    }

    pub(crate) fn is_in_auth_limbo(&self, player_id: PlayerId) -> bool {
        self.sessions.contains_key(&player_id)
    }

    pub(crate) fn is_authenticated(&self, player_id: PlayerId) -> bool {
        !self.is_in_auth_limbo(player_id)
            && self
                .connections
                .get(&player_id)
                .is_some_and(|c| c.authenticated)
    }

    fn mark_authenticated(&self, player_id: PlayerId) {
        let mut connection = self.connections.entry(player_id).or_default();
        connection.authenticated = true;
        connection.failed_attempts = 0;
    }

    pub(crate) fn record_failed_attempt(&self, player_id: PlayerId) -> Attempt {
        let mut connection = self.connections.entry(player_id).or_default();
        connection.failed_attempts += 1;
        let max = self.config.security.max_login_attempts;
        if connection.failed_attempts >= max {
            Attempt::Exhausted
        } else {
            Attempt::Retry {
                attempts_left: max - connection.failed_attempts,
            }
        }
    }

    pub(crate) fn clear_failed_attempts(&self, player_id: PlayerId) {
        if let Some(mut connection) = self.connections.get_mut(&player_id) {
            connection.failed_attempts = 0;
        }
    }

    pub fn forget_player(&self, player_id: PlayerId) {
        self.sessions.remove(&player_id);
        self.connections.remove(&player_id);
    }

    fn cleanup_session(&self, player_id: PlayerId) {
        self.sessions.remove(&player_id);
    }

    fn msg(&self, template: &str, replacements: &[(&str, &str)]) -> Component {
        let formatted = self.config.messages.format_message(template, replacements);
        parse_colored(&formatted)
    }

    fn clear_title(session: &dyn LimboSession) {
        let _ = session.send_title(
            TitleData::new(Component::text(""), Component::text(""))
                .fade_in(0)
                .stay(0)
                .fade_out(0),
        );
    }

    fn accept(&self, session: &dyn LimboSession, message: Component) {
        let _ = session.send_message(message);
        Self::clear_title(session);
        self.cleanup_session(session.player_id());
        self.mark_authenticated(session.player_id());
        session.complete(LimboOutcome::Accept);
    }

    fn spawn_reminder_task(&self, player_id: PlayerId, cancel_token: CancellationToken) {
        let interval_secs = self.config.security.title_reminder_interval_seconds;
        if interval_secs == 0 {
            return;
        }

        let player_registry = Arc::clone(&self.player_registry);
        let config = Arc::clone(&self.config);
        let cancel = cancel_token.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Some(player) = player_registry.get_player_by_id(player_id) {
                            let title = TitleData::new(
                                parse_colored(&config.messages.reminder_title),
                                parse_colored(&config.messages.reminder_subtitle),
                            )
                            .fade_in(5)
                            .stay(60)
                            .fade_out(10);
                            let _ = player.send_title(title);
                        } else {
                            break;
                        }
                    }
                    () = cancel.cancelled() => { break; }
                }
            }
        });
    }

    fn validate_password(&self, password: &str, username: &Username) -> Option<String> {
        let policy = &self.config.password_policy;

        if password.len() < policy.min_length {
            return Some(self.config.messages.format_message(
                &self.config.messages.register_password_too_short,
                &[("{min_length}", &policy.min_length.to_string())],
            ));
        }
        if password.len() > policy.max_length {
            return Some(self.config.messages.format_message(
                &self.config.messages.register_password_too_long,
                &[("{max_length}", &policy.max_length.to_string())],
            ));
        }
        if policy.check_username && password.to_lowercase() == username.as_str() {
            return Some(self.config.messages.register_password_is_username.clone());
        }
        if !self.blocked_passwords.is_empty()
            && self.blocked_passwords.contains(&password.to_lowercase())
        {
            return Some(self.config.messages.register_password_blocked.clone());
        }

        None
    }

    fn pending_session(&self, player_id: PlayerId) -> Option<(bool, Username)> {
        self.sessions
            .get(&player_id)
            .map(|entry| (entry.needs_register, entry.username.clone()))
    }

    fn send_internal_error(session: &dyn LimboSession) {
        let _ = session.send_message(Component::error(
            "An internal error occurred. Please try again.",
        ));
    }

    fn premium_login(&self, session: &dyn LimboSession) {
        let player_id = session.player_id();
        let profile = session.profile();
        let username = Username::new(&profile.username);
        let display_name = profile.username.clone();
        let mojang_uuid = profile.uuid;
        let now = chrono::Utc::now();

        match self.storage.get_account(&username) {
            Ok(Some(existing)) => {
                let first = existing
                    .premium_info
                    .as_ref()
                    .map(|pi| pi.first_premium_login)
                    .unwrap_or(now);

                let premium_info = PremiumInfo {
                    mojang_uuid,
                    force_cracked: false,
                    first_premium_login: first,
                    last_premium_login: Some(now),
                };

                if let Err(e) = self
                    .storage
                    .update_premium_info(&username, Some(premium_info))
                {
                    tracing::error!("Premium info update failed: {e}");
                }
            }
            Ok(None) => {
                let ip = self
                    .player_registry
                    .get_player_by_id(player_id)
                    .map(|p| p.remote_addr().ip());

                let premium_info = PremiumInfo {
                    mojang_uuid,
                    force_cracked: false,
                    first_premium_login: now,
                    last_premium_login: Some(now),
                };

                let account = AuthAccount {
                    username,
                    display_name: DisplayName::new(display_name.clone()),
                    password_hash: None,
                    registered_at: now,
                    last_login: Some(now),
                    last_ip: ip,
                    login_count: 1,
                    premium_info: Some(premium_info),
                };
                if let Err(e) = self.storage.create_account(&account) {
                    tracing::error!("Premium account creation failed: {e}");
                }
            }
            Err(e) => tracing::error!("Premium account lookup failed: {e}"),
        }

        let msg = self
            .config
            .premium
            .messages
            .premium_login
            .replace("{username}", &display_name);
        let _ = session.send_message(parse_colored(&msg));
        tracing::info!(%display_name, "Premium auto-login");
        self.mark_authenticated(player_id);
    }

    async fn login(&self, session: &dyn LimboSession, args: &[&str]) {
        let player_id = session.player_id();
        let Some((needs_register, username)) = self.pending_session(player_id) else {
            return;
        };
        if needs_register {
            let _ = session.send_message(self.msg(&self.config.messages.register_usage, &[]));
            return;
        }
        let Some(password) = args.first() else {
            let _ = session.send_message(self.msg(&self.config.messages.login_usage, &[]));
            return;
        };

        // Passwordless (premium) accounts verify against the dummy hash so
        // messaging and timing match unknown accounts (no account-type oracle).
        let hash = match self.storage.get_account(&username) {
            Ok(Some(AuthAccount {
                password_hash: Some(h),
                ..
            })) => h,
            Ok(_) => self.dummy_hash.clone(),
            Err(e) => {
                tracing::error!("Account lookup failed: {e}");
                Self::send_internal_error(session);
                return;
            }
        };

        match password::verify_and_migrate(password, &hash, &self.config.hashing).await {
            Ok((true, migrated_hash)) => {
                if let Some(new_hash) = migrated_hash
                    && let Err(e) = self.storage.update_password_hash(&username, new_hash)
                {
                    tracing::error!("Hash migration failed: {e}");
                }

                if let Some(player) = self.player_registry.get_player_by_id(player_id)
                    && let Err(e) = self.storage.update_last_login(
                        &username,
                        player.remote_addr().ip(),
                        chrono::Utc::now(),
                    )
                {
                    tracing::error!("Last login update failed: {e}");
                }

                self.accept(session, self.msg(&self.config.messages.login_success, &[]));
            }
            Ok((false, _)) => match self.record_failed_attempt(player_id) {
                Attempt::Exhausted => {
                    let msg = self.msg(&self.config.messages.login_max_attempts, &[]);
                    self.cleanup_session(player_id);
                    session.complete(LimboOutcome::Deny(msg));
                }
                Attempt::Retry { attempts_left } => {
                    let _ = session.send_message(self.msg(
                        &self.config.messages.login_fail,
                        &[
                            ("{attempts_left}", &attempts_left.to_string()),
                            (
                                "{max_attempts}",
                                &self.config.security.max_login_attempts.to_string(),
                            ),
                        ],
                    ));
                }
            },
            Err(e) => {
                tracing::error!("Password verification error: {e}");
                Self::send_internal_error(session);
            }
        }
    }

    async fn register(&self, session: &dyn LimboSession, args: &[&str]) {
        let player_id = session.player_id();
        let Some((needs_register, username)) = self.pending_session(player_id) else {
            return;
        };
        if !needs_register {
            let _ = session.send_message(self.msg(&self.config.messages.login_usage, &[]));
            return;
        }
        let &[password, confirm, ..] = args else {
            let _ = session.send_message(self.msg(&self.config.messages.register_usage, &[]));
            return;
        };
        if password != confirm {
            let _ = session
                .send_message(self.msg(&self.config.messages.register_password_mismatch, &[]));
            return;
        }
        if let Some(error_msg) = self.validate_password(password, &username) {
            let _ = session.send_message(parse_colored(&error_msg));
            return;
        }

        let hash = match password::hash_password(password, &self.config.hashing).await {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("Password hashing error: {e}");
                Self::send_internal_error(session);
                return;
            }
        };

        let account = AuthAccount {
            username,
            display_name: DisplayName::new(session.profile().username.clone()),
            password_hash: Some(hash),
            registered_at: chrono::Utc::now(),
            last_login: None,
            last_ip: self
                .player_registry
                .get_player_by_id(player_id)
                .map(|p| p.remote_addr().ip()),
            login_count: 0,
            premium_info: None,
        };

        match self.storage.create_account(&account) {
            Ok(()) => {
                self.accept(
                    session,
                    self.msg(&self.config.messages.register_success, &[]),
                );
            }
            Err(crate::error::AuthStorageError::AccountAlreadyExists { .. }) => {
                let _ = session
                    .send_message(self.msg(&self.config.messages.register_account_exists, &[]));
            }
            Err(e) => {
                tracing::error!("Account creation error: {e}");
                Self::send_internal_error(session);
            }
        }
    }
}

impl LimboHandler for AuthHandler {
    fn name(&self) -> &str {
        "auth"
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let player_id = session.player_id();
        let profile = session.profile();

        if self.config.premium.enabled && profile.is_mojang_authenticated() {
            self.premium_login(session);
            return Box::pin(async { HandlerResult::Accept });
        }

        let username = Username::new(&profile.username);
        let display_name = profile.username.clone();

        let needs_register = !self.storage.has_account(&username);

        if needs_register {
            let title = self.msg(
                &self.config.messages.register_title,
                &[("{username}", &display_name)],
            );
            let subtitle = self.msg(&self.config.messages.register_subtitle, &[]);
            let _ = session.send_title(
                TitleData::new(title, subtitle)
                    .fade_in(10)
                    .stay(200)
                    .fade_out(0),
            );
            let _ = session.send_message(self.msg(&self.config.messages.register_usage, &[]));
        } else {
            let title = self.msg(&self.config.messages.login_title, &[]);
            let subtitle = self.msg(&self.config.messages.login_subtitle, &[]);
            let _ = session.send_title(
                TitleData::new(title, subtitle)
                    .fade_in(10)
                    .stay(200)
                    .fade_out(0),
            );
            let _ = session.send_message(self.msg(&self.config.messages.login_usage, &[]));
        }

        self.sessions.insert(
            player_id,
            AuthSessionState {
                needs_register,
                username,
                force_completed: false,
            },
        );

        self.spawn_reminder_task(player_id, session.cancellation_token());

        tracing::debug!(player_id = ?player_id, display_name, "Player entered auth limbo");

        let timeout_secs = self.config.security.login_timeout_seconds;
        if timeout_secs == 0 {
            Box::pin(async { HandlerResult::Hold })
        } else {
            let on_timeout = LimboOutcome::Deny(parse_colored(&self.config.messages.login_timeout));
            Box::pin(async move {
                HandlerResult::HoldWithTimeout {
                    after: Duration::from_secs(timeout_secs),
                    on_timeout,
                }
            })
        }
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if self
                .sessions
                .get(&session.player_id())
                .is_some_and(|e| e.force_completed)
            {
                self.accept(session, self.msg(&self.config.messages.login_success, &[]));
                return;
            }

            // SECURITY: Never log `args` they contain passwords
            match command {
                "login" | "l" => self.login(session, args).await,
                "register" | "reg" => self.register(session, args).await,
                _ => {
                    let _ =
                        session.send_message(self.msg(&self.config.messages.unknown_command, &[]));
                }
            }
        })
    }

    fn on_chat<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        _message: &'a str,
    ) -> BoxFuture<'a, ()> {
        let player_id = session.player_id();

        if self
            .sessions
            .get(&player_id)
            .is_some_and(|e| e.force_completed)
        {
            self.accept(session, self.msg(&self.config.messages.login_success, &[]));
            return Box::pin(async {});
        }

        let _ = session.send_message(self.msg(&self.config.messages.unknown_command, &[]));
        Box::pin(async {})
    }

    fn allows_proxy_commands(&self) -> bool {
        false
    }

    fn on_session_end(&self, player_id: PlayerId, _reason: SessionEndReason) -> BoxFuture<'_, ()> {
        self.cleanup_session(player_id);
        tracing::debug!(player_id = ?player_id, "Auth session cleaned up on session end");
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use infrarust_api::test_util::MockPlayerRegistry;

    use crate::account::Username;
    use crate::error::AuthStorageError;
    use crate::test_support::{
        TestEnv, fast_config, limbo_session, limbo_session_with, premium_profile, session_text,
    };

    #[tokio::test]
    async fn register_flow_creates_account_and_accepts() {
        let env = TestEnv::new().await;
        let session = limbo_session(1, "Steve");

        let result = env.handler.on_player_enter(&*session).await;
        assert!(matches!(result, HandlerResult::HoldWithTimeout { .. }));

        env.handler
            .on_command(&*session, "register", &["hunter2hunter2", "hunter2hunter2"])
            .await;

        assert!(matches!(session.completions()[..], [LimboOutcome::Accept]));
        assert!(env.storage.has_account(&Username::new("Steve")));
    }

    #[tokio::test]
    async fn register_rejects_short_and_mismatched_passwords() {
        let env = TestEnv::new().await;
        let session = limbo_session(1, "Steve");
        env.handler.on_player_enter(&*session).await;

        env.handler
            .on_command(&*session, "register", &["short", "short"])
            .await;
        env.handler
            .on_command(&*session, "register", &["longenough1", "longenough2"])
            .await;

        assert!(session.completions().is_empty());
        assert!(!env.storage.has_account(&Username::new("Steve")));
    }

    #[tokio::test]
    async fn login_flow_accepts_correct_password() {
        let env = TestEnv::new().await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        let session = limbo_session(1, "Steve");

        let result = env.handler.on_player_enter(&*session).await;
        assert!(matches!(result, HandlerResult::HoldWithTimeout { .. }));

        env.handler
            .on_command(&*session, "login", &["hunter2hunter2"])
            .await;

        assert!(matches!(session.completions()[..], [LimboOutcome::Accept]));
    }

    #[tokio::test]
    async fn wrong_passwords_deny_after_max_attempts() {
        let mut config = fast_config();
        config.security.max_login_attempts = 2;
        let env = TestEnv::with_config(config).await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        let session = limbo_session(1, "Steve");
        env.handler.on_player_enter(&*session).await;

        env.handler
            .on_command(&*session, "login", &["wrong-password"])
            .await;
        assert!(session.completions().is_empty());

        env.handler
            .on_command(&*session, "login", &["wrong-password"])
            .await;
        assert!(matches!(session.completions()[..], [LimboOutcome::Deny(_)]));
    }

    #[tokio::test]
    async fn force_complete_bypasses_password_on_next_input() {
        let env = TestEnv::new().await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        let session = limbo_session(1, "Steve");
        env.handler.on_player_enter(&*session).await;

        assert!(env.handler.force_complete_session(PlayerId::new(1)));

        env.handler.on_chat(&*session, "hello").await;
        assert!(matches!(session.completions()[..], [LimboOutcome::Accept]));
    }

    #[tokio::test]
    async fn premium_auto_login_accepts_and_creates_account() {
        let mut config = fast_config();
        config.premium.enabled = true;
        let env = TestEnv::with_config(config).await;
        let session = limbo_session_with(1, premium_profile(1, "Notch"));

        let result = env.handler.on_player_enter(&*session).await;
        assert!(matches!(result, HandlerResult::Accept));

        let account = env
            .storage
            .get_account(&Username::new("Notch"))
            .unwrap()
            .expect("the account exists as soon as the handler returns");
        assert!(account.premium_info.is_some());
        assert!(account.password_hash.is_none());
    }

    struct UnreadableStorage {
        writes: AtomicUsize,
    }

    impl UnreadableStorage {
        fn unreadable() -> AuthStorageError {
            AuthStorageError::Io(std::io::Error::other("backend unreachable"))
        }

        fn written(&self) -> Result<(), AuthStorageError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    impl AuthStorage for UnreadableStorage {
        fn has_account(&self, _: &Username) -> bool {
            true
        }

        fn get_account(&self, _: &Username) -> Result<Option<AuthAccount>, AuthStorageError> {
            Err(Self::unreadable())
        }

        fn is_force_cracked(&self, _: &Username) -> bool {
            false
        }

        fn create_account(&self, _: &AuthAccount) -> Result<(), AuthStorageError> {
            self.written()
        }

        fn update_password_hash(
            &self,
            _: &Username,
            _: crate::account::PasswordHash,
        ) -> Result<(), AuthStorageError> {
            self.written()
        }

        fn delete_account(&self, _: &Username) -> Result<bool, AuthStorageError> {
            self.written().map(|()| true)
        }

        fn update_last_login(
            &self,
            _: &Username,
            _: std::net::IpAddr,
            _: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), AuthStorageError> {
            self.written()
        }

        fn update_premium_info(
            &self,
            _: &Username,
            _: Option<PremiumInfo>,
        ) -> Result<(), AuthStorageError> {
            self.written()
        }

        fn flush(&self) -> BoxFuture<'_, Result<(), AuthStorageError>> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn handler_over_unreadable_storage(
        config: AuthConfig,
    ) -> (AuthHandler, Arc<UnreadableStorage>) {
        let storage = Arc::new(UnreadableStorage {
            writes: AtomicUsize::new(0),
        });
        let dummy_hash = password::generate_dummy_hash(&config.hashing)
            .await
            .unwrap();
        let handler = AuthHandler::new(
            Arc::clone(&storage) as Arc<dyn AuthStorage>,
            Arc::new(config),
            Arc::new(MockPlayerRegistry::new()),
            dummy_hash,
            HashSet::new(),
            None,
        );
        (handler, storage)
    }

    #[tokio::test]
    async fn premium_login_does_not_create_an_account_when_the_lookup_fails() {
        let mut config = fast_config();
        config.premium.enabled = true;
        let (handler, storage) = handler_over_unreadable_storage(config).await;
        let session = limbo_session_with(1, premium_profile(1, "Notch"));

        let result = handler.on_player_enter(&*session).await;
        tokio::task::yield_now().await;

        assert!(matches!(result, HandlerResult::Accept));
        assert_eq!(storage.writes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn login_reports_a_storage_error_instead_of_a_wrong_password() {
        let (handler, _storage) = handler_over_unreadable_storage(fast_config()).await;
        let session = limbo_session(1, "Steve");
        handler.on_player_enter(&*session).await;

        handler
            .on_command(&*session, "login", &["hunter2hunter2"])
            .await;

        let text = session_text(&session);
        assert!(text.contains("internal error"), "got: {text}");
        assert!(!text.contains("Wrong password"), "got: {text}");
        assert!(session.completions().is_empty());
    }

    #[tokio::test]
    async fn passwordless_account_login_matches_unknown_account_messaging() {
        let env = TestEnv::new().await;
        env.create_account("Notch", None).await;
        env.set_premium_info("Notch", false).await;
        let session = limbo_session(1, "Notch");
        env.handler.on_player_enter(&*session).await;

        env.handler
            .on_command(&*session, "login", &["whatever-password"])
            .await;

        let text = session_text(&session);
        assert!(text.contains("Wrong password"), "got: {text}");
        assert!(!text.to_lowercase().contains("premium"), "got: {text}");
        assert!(session.completions().is_empty());
    }

    #[tokio::test]
    async fn a_login_lasts_until_the_player_disconnects() {
        let env = TestEnv::new().await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        env.log_in(1, "Steve", "hunter2hunter2").await;
        assert!(env.handler.is_authenticated(PlayerId::new(1)));

        env.handler.forget_player(PlayerId::new(1));
        assert!(!env.handler.is_authenticated(PlayerId::new(1)));
    }

    #[tokio::test]
    async fn session_end_cleans_up_state() {
        let env = TestEnv::new().await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        let session = limbo_session(1, "Steve");
        env.handler.on_player_enter(&*session).await;
        assert!(env.handler.is_in_auth_limbo(PlayerId::new(1)));

        env.handler
            .on_session_end(PlayerId::new(1), SessionEndReason::Disconnected)
            .await;
        assert!(!env.handler.is_in_auth_limbo(PlayerId::new(1)));
    }

    #[tokio::test]
    async fn the_registered_handler_forwards_session_callbacks_to_the_shared_handler() {
        let env = TestEnv::new().await;
        env.create_account("Steve", Some("hunter2hunter2")).await;
        let registered: Box<dyn LimboHandler> = Box::new(Arc::clone(&env.handler));
        let session = limbo_session(1, "Steve");

        assert_eq!(registered.name(), "auth");
        assert!(!registered.allows_proxy_commands());
        registered.on_player_enter(&*session).await;
        assert!(env.handler.is_in_auth_limbo(PlayerId::new(1)));

        registered.on_disconnect(PlayerId::new(1)).await;
        registered
            .on_session_end(PlayerId::new(1), SessionEndReason::Disconnected)
            .await;
        assert!(!env.handler.is_in_auth_limbo(PlayerId::new(1)));
    }
}
