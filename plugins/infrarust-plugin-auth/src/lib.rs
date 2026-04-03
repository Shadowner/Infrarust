//! Password-based `/login` and `/register` authentication plugin for Infrarust V2.

pub mod account;
pub mod commands;
pub mod config;
pub mod error;
pub mod handler;
pub mod ip_mask;
pub mod password;
pub mod premium;
pub mod storage;
pub mod util;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::types::PlayerId;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::config::load_or_create_config;
use crate::handler::AuthHandler;
use crate::storage::AuthStorage;
use crate::storage::json::JsonFileStorage;

struct PluginState {
    storage: Arc<dyn AuthStorage>,
    save_cancel: CancellationToken,
}

pub struct AuthPlugin {
    state: Mutex<Option<PluginState>>,
}

impl AuthPlugin {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }
}

impl Default for AuthPlugin {
    fn default() -> Self {
        Self::new()
    }
}

async fn load_blocked_passwords(data_dir: &std::path::Path, filename: &str) -> HashSet<String> {
    if filename.is_empty() {
        return HashSet::new();
    }

    let path = data_dir.join(filename);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let set: HashSet<String> = content
                .lines()
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            tracing::info!(count = set.len(), path = %path.display(), "Loaded blocked passwords");
            set
        }
        Err(_) => {
            tracing::debug!(
                path = %path.display(),
                "No blocked passwords file found — password blocklist disabled"
            );
            HashSet::new()
        }
    }
}

impl Plugin for AuthPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("auth", "Auth Plugin", "1.0.0")
            .author("Infrarust")
            .description("Password-based authentication for offline-mode proxies")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            let data_dir = ctx.data_dir();
            let config_path = data_dir.join("config.toml");

            let config = load_or_create_config(&config_path)
                .await
                .map_err(|e| PluginError::InitFailed(e.to_string()))?;
            let config = Arc::new(config);

            password::validate_hashing_config(&config.hashing)
                .map_err(|e| PluginError::InitFailed(e.to_string()))?;

            let storage: Arc<dyn AuthStorage> = Arc::new(
                JsonFileStorage::load_or_create(&data_dir, &config.storage.path)
                    .await
                    .map_err(|e| PluginError::InitFailed(e.to_string()))?,
            );

            let dummy_hash = password::generate_dummy_hash(&config.hashing)
                .await
                .map_err(|e| PluginError::InitFailed(e.to_string()))?;

            let blocked_passwords =
                load_blocked_passwords(&data_dir, &config.password_policy.blocked_passwords_file)
                    .await;

            let premium_cache = if config.premium.enabled {
                let cache = Arc::new(premium::PremiumCache::new(
                    std::time::Duration::from_secs(config.premium.cache_ttl_seconds),
                    std::time::Duration::from_secs(config.premium.failed_auth_remember_seconds),
                ));
                let lookup = Arc::new(premium::MojangApiLookup::new(
                    config.premium.rate_limit_per_second,
                ));
                let detector = Arc::new(premium::PremiumDetector::new(
                    Arc::clone(&cache),
                    lookup,
                    Arc::clone(&storage),
                    Arc::new(config.premium.clone()),
                ));

                ctx.event_bus()
                    .subscribe_async::<infrarust_api::events::lifecycle::PreLoginEvent, _>(
                        infrarust_api::event::EventPriority::EARLY,
                        detector.pre_login_handler(),
                    );

                let failure_cache = Arc::clone(&cache);
                ctx.event_bus().subscribe::<
                    infrarust_api::events::lifecycle::OnlineAuthFailed, _
                >(
                    infrarust_api::event::EventPriority::NORMAL,
                    move |event| {
                        failure_cache.mark_auth_failed(&event.username);
                        tracing::info!(
                            username = %event.username,
                            "Remembered failed premium auth — next attempt will skip ForceOnline"
                        );
                    },
                );

                tracing::info!("[AuthPlugin] Premium auto-login enabled");
                Some(cache)
            } else {
                None
            };

            let handler = Arc::new(AuthHandler::new(
                Arc::clone(&storage),
                Arc::clone(&config),
                ctx.player_registry_handle(),
                dummy_hash,
                blocked_passwords,
                premium_cache,
            ));

            ctx.register_limbo_handler(Box::new(AuthLimbo(Arc::clone(&handler))));
            commands::register_commands(ctx, Arc::clone(&handler));

            let save_cancel = CancellationToken::new();
            let save_storage = Arc::clone(&storage);
            let save_interval = config.storage.auto_save_interval_seconds;
            let save_token = save_cancel.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(save_interval));
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            if let Err(e) = save_storage.flush().await {
                                tracing::error!("Auth auto-save failed: {e}");
                            }
                        }
                        () = save_token.cancelled() => { break; }
                    }
                }
            });

            let mut guard = self.state.lock().expect("auth plugin state mutex poisoned");
            *guard = Some(PluginState {
                storage,
                save_cancel,
            });

            tracing::info!("[AuthPlugin] Enabled — limbo handler 'auth' registered");
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        let state = self
            .state
            .lock()
            .expect("auth plugin state mutex poisoned")
            .take();

        Box::pin(async move {
            if let Some(state) = state {
                state.save_cancel.cancel();
                if let Err(e) = state.storage.flush().await {
                    tracing::error!("Auth final flush failed: {e}");
                }
            }
            tracing::info!("[AuthPlugin] Disabled");
            Ok(())
        })
    }
}

struct AuthLimbo(Arc<AuthHandler>);

impl std::ops::Deref for AuthLimbo {
    type Target = AuthHandler;
    fn deref(&self) -> &AuthHandler {
        &self.0
    }
}

impl LimboHandler for AuthLimbo {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        (**self).on_player_enter(session)
    }

    fn on_command<'a>(
        &'a self,
        session: &'a dyn LimboSession,
        command: &'a str,
        args: &'a [&'a str],
    ) -> BoxFuture<'a, ()> {
        (**self).on_command(session, command, args)
    }

    fn on_chat<'a>(&'a self, session: &'a dyn LimboSession, message: &'a str) -> BoxFuture<'a, ()> {
        (**self).on_chat(session, message)
    }

    fn on_disconnect(&self, player_id: PlayerId) -> BoxFuture<'_, ()> {
        (**self).on_disconnect(player_id)
    }
}
