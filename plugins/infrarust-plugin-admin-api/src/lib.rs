pub mod api_provider;
pub mod auth;
pub mod config;
pub mod drain_store;
pub mod dto;
pub mod error;
pub mod frontend;
pub mod handlers;
pub mod health_cache;
pub mod health_checker;
pub mod log_layer;
pub mod rate_limit;
pub mod response;
pub mod router;
pub mod server_dir;
pub mod sse;
pub mod state;
pub mod util;

use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::api_provider::ApiConfigProvider;
use crate::config::ApiConfig;
use crate::drain_store::DrainStore;
use crate::health_cache::HealthCache;
use crate::health_checker::HealthChecker;
use crate::log_layer::LogBroadcast;
use crate::rate_limit::RateLimiter;
use crate::router::build_router;
use crate::server_dir::{ProviderSenderSlot, ServerDir};
use crate::sse::event_bridge::EventBridge;
use crate::sse::stats_ticker::StatsTicker;
use crate::state::{ApiEvent, ApiState};

const EVENT_CHANNEL_CAPACITY: usize = 256;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AdminApiPlugin {
    tasks: Mutex<JoinSet<()>>,
    shutdown: CancellationToken,
    config: Mutex<Option<ApiConfig>>,
    enable_webui: bool,
    logs: Option<LogBroadcast>,
}

impl AdminApiPlugin {
    pub fn new(config: ApiConfig, enable_webui: bool, logs: Option<LogBroadcast>) -> Self {
        Self {
            tasks: Mutex::new(JoinSet::new()),
            shutdown: CancellationToken::new(),
            config: Mutex::new(Some(config)),
            enable_webui,
            logs,
        }
    }

    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(task);
    }

    fn take_config(&self) -> Result<ApiConfig, PluginError> {
        let mut config = self
            .config
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| PluginError::InitFailed("Config already consumed".into()))?;

        config.cors_origins.retain(|origin| {
            if origin.parse::<axum::http::HeaderValue>().is_err() {
                tracing::warn!(origin = %origin, "Ignoring invalid CORS origin");
                false
            } else {
                true
            }
        });
        Ok(config)
    }

    fn spawn_server_dir(
        &self,
        ctx: &dyn PluginContext,
        data_dir: &Path,
    ) -> Result<(Arc<ServerDir>, Arc<ProviderSenderSlot>), PluginError> {
        let server_dir = Arc::new(ServerDir::open(data_dir).map_err(|e| {
            PluginError::InitFailed(format!("Failed to open the servers directory: {e}"))
        })?);
        let provider_sender = Arc::new(tokio::sync::Mutex::new(None));

        ctx.register_config_provider(Box::new(ApiConfigProvider {
            dir: server_dir.clone(),
            sender: provider_sender.clone(),
            shutdown: self.shutdown.clone(),
        }));

        self.spawn(server_dir::watch(
            server_dir.clone(),
            provider_sender.clone(),
            self.shutdown.clone(),
        ));
        Ok((server_dir, provider_sender))
    }

    fn spawn_drain_reapply(&self, ctx: &dyn PluginContext, data_dir: &Path) -> Arc<DrainStore> {
        let drain_store = Arc::new(DrainStore::open(data_dir));
        self.spawn(drain_store::reapply(
            drain_store.clone(),
            ctx.load_balancer_service_handle(),
            self.shutdown.clone(),
        ));
        drain_store
    }

    fn spawn_event_bridge(&self, ctx: &dyn PluginContext, state: &ApiState) {
        EventBridge::new(state.event_tx.clone()).register_listeners(ctx);

        let recent = state.recent_events.clone();
        let mut rx = state.event_tx.subscribe();
        let shutdown = self.shutdown.clone();
        self.spawn(async move {
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    result = rx.recv() => match result {
                        Ok(event) => state::push_recent_event(&recent, &event),
                        Err(broadcast::error::RecvError::Lagged(_)) => {},
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
    }

    fn spawn_stats_ticker(&self, ctx: &dyn PluginContext, state: &ApiState) {
        let ticker = StatsTicker::new(
            state.event_tx.clone(),
            ctx.player_registry_handle(),
            ctx.server_manager_handle(),
            ctx.ban_service_handle(),
            state.start_time,
            self.shutdown.clone(),
        );
        self.spawn(ticker.run());
    }

    async fn spawn_http_server(&self, state: Arc<ApiState>) -> Result<(), PluginError> {
        let bind = state.config.bind.clone();
        let app = build_router(state, self.enable_webui);

        let listener = tokio::net::TcpListener::bind(&bind).await.map_err(|e| {
            PluginError::InitFailed(format!("Failed to bind admin API on {bind}: {e}"))
        })?;

        tracing::info!(bind = %bind, "Admin API server starting");

        let shutdown = self.shutdown.clone();
        self.spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            {
                tracing::error!(error = %e, "Admin API server error");
            }
        });
        Ok(())
    }
}

async fn join_all(mut tasks: JoinSet<()>) {
    let deadline = tokio::time::sleep(SHUTDOWN_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            joined = tasks.join_next() => match joined {
                None => break,
                Some(Ok(())) => {}
                Some(Err(e)) => tracing::error!(error = %e, "Admin API task panicked"),
            },
            () = &mut deadline => {
                tracing::warn!(
                    pending = tasks.len(),
                    "Admin API tasks did not stop within {SHUTDOWN_TIMEOUT:?}, aborting them"
                );
                tasks.abort_all();
                break;
            }
        }
    }
}

impl Plugin for AdminApiPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("admin_api", "Admin REST API", env!("CARGO_PKG_VERSION"))
            .author("Infrarust Team")
            .description("HTTP REST API for proxy administration and monitoring")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            let data_dir = ctx.data_dir();
            let config = self.take_config()?;

            if self.logs.is_none() {
                tracing::info!("No log broadcast: /api/v1/logs and /api/v1/logs/history are off");
            }

            let (event_tx, _) = broadcast::channel::<ApiEvent>(EVENT_CHANNEL_CAPACITY);
            let (server_dir, provider_sender) = self.spawn_server_dir(ctx, &data_dir)?;
            let drain_store = self.spawn_drain_reapply(ctx, &data_dir);

            let state = Arc::new(ApiState {
                player_registry: ctx.player_registry_handle(),
                ban_service: ctx.ban_service_handle(),
                server_manager: ctx.server_manager_handle(),
                config_service: ctx.config_service_handle(),
                load_balancer: ctx.load_balancer_service_handle(),
                plugin_registry: ctx.plugin_registry_handle(),
                rate_limiter: RateLimiter::new(config.rate_limit.requests_per_minute),
                config,
                start_time: Instant::now(),
                proxy_version: env!("CARGO_PKG_VERSION").into(),
                event_tx,
                shutdown: self.shutdown.clone(),
                proxy_shutdown: ctx.proxy_shutdown(),
                logs: self.logs.clone(),
                server_dir,
                provider_sender,
                health_cache: Arc::new(HealthCache::new()),
                health_checker: Arc::new(HealthChecker::new()),
                recent_events: Arc::new(Mutex::new(VecDeque::new())),
                drain_store,
            });

            self.spawn_event_bridge(ctx, &state);
            self.spawn_stats_ticker(ctx, &state);
            self.spawn_http_server(state).await
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        self.shutdown.cancel();

        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|p| p.into_inner()));

        Box::pin(async move {
            join_all(tasks).await;
            tracing::info!("Admin API server stopped");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::Arc;
    use std::time::Instant;

    use infrarust_api::plugin::Plugin;
    use infrarust_api::test_util::{MockBanService, MockPlayerRegistry, MockServerManager};
    use tokio::sync::broadcast;
    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;
    use crate::config::{ApiConfig, RateLimitConfig};

    #[tokio::test]
    async fn disabling_stops_the_stats_ticker_before_returning() {
        let config = ApiConfig {
            bind: "127.0.0.1:0".into(),
            api_key: "key".into(),
            cors_origins: vec![],
            rate_limit: RateLimitConfig::default(),
        };
        let plugin = AdminApiPlugin::new(config, false, None);
        let (event_tx, mut events) = broadcast::channel::<ApiEvent>(16);
        let ticker = StatsTicker::new(
            event_tx,
            Arc::new(MockPlayerRegistry::new()),
            Arc::new(MockServerManager::new()),
            Arc::new(MockBanService::new()),
            Instant::now(),
            plugin.shutdown.clone(),
        );
        plugin.spawn(ticker.run());
        events.recv().await.unwrap();

        plugin.on_disable().await.unwrap();

        let closed = loop {
            match events.try_recv() {
                Ok(_) | Err(TryRecvError::Lagged(_)) => {}
                Err(other) => break other,
            }
        };
        assert!(matches!(closed, TryRecvError::Closed), "{closed:?}");
    }
}
