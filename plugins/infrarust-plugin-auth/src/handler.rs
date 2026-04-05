//! Auth limbo handler — the core authentication flow.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
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
    failed_attempts: u32,
    needs_register: bool,
    username: Username,
    cancel_token: CancellationToken,
    force_completed: bool,
}

pub struct AuthHandler {
    sessions: DashMap<PlayerId, AuthSessionState>,
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

    pub(crate) fn force_complete_session(&self, player_id: PlayerId) -> bool {
        if let Some(mut entry) = self.sessions.get_mut(&player_id) {
            entry.force_completed = true;
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub(crate) fn is_in_auth_limbo(&self, player_id: PlayerId) -> bool {
        self.sessions.contains_key(&player_id)
    }

    fn cleanup_session(&self, player_id: PlayerId) {
        if let Some((_, state)) = self.sessions.remove(&player_id) {
            state.cancel_token.cancel();
        }
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

    fn spawn_timeout_task(&self, player_id: PlayerId, cancel_token: CancellationToken) {
        let timeout_secs = self.config.security.login_timeout_seconds;
        if timeout_secs == 0 {
            return;
        }

        let player_registry = Arc::clone(&self.player_registry);
        let config = Arc::clone(&self.config);
        let cancel = cancel_token.clone();

        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(timeout_secs)) => {
                    if let Some(player) = player_registry.get_player_by_id(player_id) {
                        player.disconnect(parse_colored(&config.messages.login_timeout)).await;
                    }
                }
                () = cancel.cancelled() => {}
            }
        });
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
            let username = Username::new(&profile.username);
            let display_name = profile.username.clone();
            let mojang_uuid = profile.uuid;
            let now = chrono::Utc::now();

            match self.storage.get_account_blocking(&username) {
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

                    let storage = Arc::clone(&self.storage);
                    let u = username.clone();
                    tokio::spawn(async move {
                        if let Err(e) = storage.update_premium_info(&u, Some(premium_info)).await {
                            tracing::error!("Premium info update failed: {e}");
                        }
                    });
                }
                _ => {
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
                        username: username.clone(),
                        display_name: DisplayName::new(display_name.clone()),
                        password_hash: None,
                        registered_at: now,
                        last_login: Some(now),
                        last_ip: ip,
                        login_count: 1,
                        premium_info: Some(premium_info),
                    };
                    let storage = Arc::clone(&self.storage);
                    tokio::spawn(async move {
                        if let Err(e) = storage.create_account(&account).await {
                            tracing::error!("Premium account creation failed: {e}");
                        }
                    });
                }
            }

            let msg = self
                .config
                .premium
                .messages
                .premium_login
                .replace("{username}", &display_name);
            let _ = session.send_message(parse_colored(&msg));
            tracing::info!(%display_name, "Premium auto-login");
            return Box::pin(async { HandlerResult::Accept });
        }

        let username = Username::new(&profile.username);
        let display_name = profile.username.clone();
        let cancel_token = CancellationToken::new();

        let needs_register = !self.storage.has_account_blocking(&username);

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
                failed_attempts: 0,
                needs_register,
                username,
                cancel_token: cancel_token.clone(),
                force_completed: false,
            },
        );

        self.spawn_timeout_task(player_id, cancel_token.clone());
        self.spawn_reminder_task(player_id, cancel_token);

        tracing::debug!(player_id = ?player_id, display_name, "Player entered auth limbo");

        Box::pin(async { HandlerResult::Hold })
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        let player_id = session.player_id();

        if self
            .sessions
            .get(&player_id)
            .is_some_and(|e| e.force_completed)
        {
            let _ = session.send_message(self.msg(&self.config.messages.login_success, &[]));
            Self::clear_title(session);
            self.cleanup_session(player_id);
            session.complete(HandlerResult::Accept);
            return Box::pin(async {});
        }

        // SECURITY: Never log `args` they contain passwords
        match command {
            "login" | "l" => {
                let Some(entry) = self.sessions.get(&player_id) else {
                    return Box::pin(async {});
                };
                if entry.needs_register {
                    drop(entry);
                    let _ =
                        session.send_message(self.msg(&self.config.messages.register_usage, &[]));
                    return Box::pin(async {});
                }
                let username = entry.username.clone();
                drop(entry);

                let Some(password) = args.first() else {
                    let _ = session.send_message(self.msg(&self.config.messages.login_usage, &[]));
                    return Box::pin(async {});
                };
                let password = password.to_string();

                let hash = match self.storage.get_account_blocking(&username) {
                    Ok(Some(account)) => match account.password_hash {
                        Some(h) => h.clone(),
                        None => {
                            let msg = if account.premium_info.is_some() {
                                "This is a premium account. Use your official Minecraft launcher."
                            } else {
                                "This account has no password set. Contact an administrator."
                            };
                            let _ = session.send_message(Component::error(msg));
                            return Box::pin(async {});
                        }
                    },
                    _ => self.dummy_hash.clone(),
                };

                let storage = Arc::clone(&self.storage);
                let player_registry = Arc::clone(&self.player_registry);

                Box::pin(async move {
                    let result =
                        password::verify_and_migrate(&password, &hash, &self.config.hashing).await;

                    match result {
                        Ok((true, migrated_hash)) => {
                            if let Some(new_hash) = migrated_hash {
                                let s = Arc::clone(&storage);
                                let u = username.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = s.update_password_hash(&u, new_hash).await {
                                        tracing::error!("Hash migration failed: {e}");
                                    }
                                });
                            }

                            if let Some(player) = player_registry.get_player_by_id(player_id) {
                                let ip = player.remote_addr().ip();
                                let s = Arc::clone(&storage);
                                let u = username.clone();
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        s.update_last_login(&u, ip, chrono::Utc::now()).await
                                    {
                                        tracing::error!("Last login update failed: {e}");
                                    }
                                });
                            }

                            let _ = session
                                .send_message(self.msg(&self.config.messages.login_success, &[]));
                            Self::clear_title(session);
                            self.cleanup_session(player_id);
                            session.complete(HandlerResult::Accept);
                        }
                        Ok((false, _)) => {
                            let (should_kick, attempts_left) =
                                if let Some(mut entry) = self.sessions.get_mut(&player_id) {
                                    entry.failed_attempts += 1;
                                    let left = self
                                        .config
                                        .security
                                        .max_login_attempts
                                        .saturating_sub(entry.failed_attempts);
                                    (
                                        entry.failed_attempts
                                            >= self.config.security.max_login_attempts,
                                        left,
                                    )
                                } else {
                                    (false, 0)
                                };

                            if should_kick {
                                let msg = self.msg(&self.config.messages.login_max_attempts, &[]);
                                self.cleanup_session(player_id);
                                session.complete(HandlerResult::Deny(msg));
                            } else {
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
                        }
                        Err(e) => {
                            tracing::error!("Password verification error: {e}");
                            let _ = session.send_message(Component::error(
                                "An internal error occurred. Please try again.",
                            ));
                        }
                    }
                })
            }

            "register" | "reg" => {
                let Some(entry) = self.sessions.get(&player_id) else {
                    return Box::pin(async {});
                };
                if !entry.needs_register {
                    drop(entry);
                    let _ = session.send_message(self.msg(&self.config.messages.login_usage, &[]));
                    return Box::pin(async {});
                }
                let username = entry.username.clone();
                let display_name = session.profile().username.clone();
                drop(entry);

                if args.len() < 2 {
                    let _ =
                        session.send_message(self.msg(&self.config.messages.register_usage, &[]));
                    return Box::pin(async {});
                }

                let password = args[0].to_string();
                let confirm = args[1].to_string();

                if password != confirm {
                    let _ = session.send_message(
                        self.msg(&self.config.messages.register_password_mismatch, &[]),
                    );
                    return Box::pin(async {});
                }

                if let Some(error_msg) = self.validate_password(&password, &username) {
                    let _ = session.send_message(parse_colored(&error_msg));
                    return Box::pin(async {});
                }

                let storage = Arc::clone(&self.storage);
                let player_registry = Arc::clone(&self.player_registry);

                Box::pin(async move {
                    let hash = match password::hash_password(&password, &self.config.hashing).await
                    {
                        Ok(h) => h,
                        Err(e) => {
                            tracing::error!("Password hashing error: {e}");
                            let _ = session.send_message(Component::error(
                                "An internal error occurred. Please try again.",
                            ));
                            return;
                        }
                    };

                    let account = AuthAccount {
                        username: username.clone(),
                        display_name: DisplayName::new(display_name),
                        password_hash: Some(hash),
                        registered_at: chrono::Utc::now(),
                        last_login: None,
                        last_ip: player_registry
                            .get_player_by_id(player_id)
                            .map(|p| p.remote_addr().ip()),
                        login_count: 0,
                        premium_info: None,
                    };

                    match storage.create_account(&account).await {
                        Ok(()) => {
                            let _ = session.send_message(
                                self.msg(&self.config.messages.register_success, &[]),
                            );
                            Self::clear_title(session);
                            self.cleanup_session(player_id);
                            session.complete(HandlerResult::Accept);
                        }
                        Err(crate::error::AuthStorageError::AccountAlreadyExists { .. }) => {
                            let _ = session.send_message(
                                self.msg(&self.config.messages.register_account_exists, &[]),
                            );
                        }
                        Err(e) => {
                            tracing::error!("Account creation error: {e}");
                            let _ = session.send_message(Component::error(
                                "An internal error occurred. Please try again.",
                            ));
                        }
                    }
                })
            }

            _ => {
                let _ = session.send_message(self.msg(&self.config.messages.unknown_command, &[]));
                Box::pin(async {})
            }
        }
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
            let _ = session.send_message(self.msg(&self.config.messages.login_success, &[]));
            Self::clear_title(session);
            self.cleanup_session(player_id);
            session.complete(HandlerResult::Accept);
            return Box::pin(async {});
        }

        let _ = session.send_message(self.msg(&self.config.messages.unknown_command, &[]));
        Box::pin(async {})
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        self.cleanup_session(player_id);
        tracing::debug!(player_id = ?player_id, "Auth session cleaned up on disconnect");
        Box::pin(async {})
    }
}
