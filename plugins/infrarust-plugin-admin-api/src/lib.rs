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

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::api_provider::ApiConfigProvider;
use crate::config::ApiConfig;
use crate::drain_store::DrainStore;
use crate::health_cache::HealthCache;
use crate::health_checker::HealthChecker;
use crate::log_layer::LogBroadcast;
use crate::rate_limit::RateLimiter;

const EVENT_CHANNEL_CAPACITY: usize = 256;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
use crate::router::build_router;
use crate::server_dir::ServerDir;
use crate::sse::event_bridge::EventBridge;
use crate::sse::stats_ticker::StatsTicker;
use crate::state::{ApiEvent, ApiState};

pub struct AdminApiPlugin {
    server_handle: Mutex<Option<JoinHandle<()>>>,
    shutdown: CancellationToken,
    config: Mutex<Option<ApiConfig>>,
    enable_webui: bool,
    logs: Option<LogBroadcast>,
}

impl AdminApiPlugin {
    pub fn new(config: ApiConfig, enable_webui: bool, logs: Option<LogBroadcast>) -> Self {
        Self {
            server_handle: Mutex::new(None),
            shutdown: CancellationToken::new(),
            config: Mutex::new(Some(config)),
            enable_webui,
            logs,
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

            let (event_tx, _) = broadcast::channel::<ApiEvent>(EVENT_CHANNEL_CAPACITY);

            let rate_limiter = RateLimiter::new(config.rate_limit.requests_per_minute);

            if self.logs.is_none() {
                tracing::info!("No log broadcast: /api/v1/logs and /api/v1/logs/history are off");
            }

            // Register API config provider for dynamic server management
            let server_dir = Arc::new(ServerDir::open(&data_dir).map_err(|e| {
                PluginError::InitFailed(format!("Failed to open the servers directory: {e}"))
            })?);
            let provider_sender = Arc::new(tokio::sync::Mutex::new(None));

            let provider = ApiConfigProvider {
                dir: server_dir.clone(),
                sender: provider_sender.clone(),
                shutdown: self.shutdown.clone(),
            };
            ctx.register_config_provider(Box::new(provider));

            tokio::spawn(server_dir::watch(
                server_dir.clone(),
                provider_sender.clone(),
                self.shutdown.clone(),
            ));

            let start_time = Instant::now();

            let drain_store = Arc::new(DrainStore::open(&data_dir));
            tokio::spawn(drain_store::reapply(
                drain_store.clone(),
                ctx.load_balancer_service_handle(),
                self.shutdown.clone(),
            ));

            let state = Arc::new(ApiState {
                player_registry: ctx.player_registry_handle(),
                ban_service: ctx.ban_service_handle(),
                server_manager: ctx.server_manager_handle(),
                config_service: ctx.config_service_handle(),
                load_balancer: ctx.load_balancer_service_handle(),
                plugin_registry: ctx.plugin_registry_handle(),
                config: config.clone(),
                start_time,
                proxy_version: env!("CARGO_PKG_VERSION").into(),
                rate_limiter,
                event_tx: event_tx.clone(),
                shutdown: self.shutdown.clone(),
                proxy_shutdown: ctx.proxy_shutdown(),
                logs: self.logs.clone(),
                server_dir,
                provider_sender,
                health_cache: Arc::new(HealthCache::new()),
                health_checker: Arc::new(HealthChecker::new()),
                recent_events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                drain_store,
            });

            // Wire up EventBridge: proxy EventBus → broadcast::Sender<ApiEvent>
            let bridge = EventBridge::new(event_tx.clone());
            bridge.register_listeners(ctx);

            // Spawn recent-events buffer: reads broadcast and stores last 100 events
            {
                let recent = state.recent_events.clone();
                let mut rx = state.event_tx.subscribe();
                let shutdown = self.shutdown.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => break,
                            result = rx.recv() => {
                                match result {
                                    Ok(event) => state::push_recent_event(&recent, &event),
                                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                                    Err(_) => break,
                                }
                            }
                        }
                    }
                });
            }

            // Spawn StatsTicker: periodic stats every 5 seconds
            let ticker = StatsTicker::new(
                event_tx,
                ctx.player_registry_handle(),
                ctx.server_manager_handle(),
                ctx.ban_service_handle(),
                start_time,
                self.shutdown.clone(),
            );
            tokio::spawn(ticker.run());

            let app = build_router(state, self.enable_webui);

            let listener = tokio::net::TcpListener::bind(&config.bind)
                .await
                .map_err(|e| {
                    PluginError::InitFailed(format!(
                        "Failed to bind admin API on {}: {e}",
                        config.bind
                    ))
                })?;

            tracing::info!(bind = %config.bind, "Admin API server starting");

            let shutdown = self.shutdown.clone();
            let handle = tokio::spawn(async move {
                // ConnectInfo exposes the peer address for per-IP rate limiting.
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

            *self.server_handle.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);

            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        self.shutdown.cancel();

        let handle = self
            .server_handle
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();

        Box::pin(async move {
            if let Some(handle) = handle {
                match tokio::time::timeout(SHUTDOWN_TIMEOUT, handle).await {
                    Ok(Ok(())) => {}
                    Ok(Err(join_err)) => {
                        tracing::error!(error = %join_err, "Admin API server task panicked");
                    }
                    Err(_) => {
                        tracing::warn!("Admin API server did not shut down within 5 seconds");
                    }
                }
            }
            tracing::info!("Admin API server stopped");
            Ok(())
        })
    }
}
