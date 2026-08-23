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
}

impl AdminApiPlugin {
    pub fn new(config: ApiConfig, enable_webui: bool) -> Self {
        Self {
            server_handle: Mutex::new(None),
            shutdown: CancellationToken::new(),
            config: Mutex::new(Some(config)),
            enable_webui,
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

            // Retrieve the log broadcast from the global singleton (set by main.rs)
            let (log_tx, log_history) = match LogBroadcast::get() {
                Some(lb) => (Some(lb.tx.clone()), Some(lb.history.clone())),
                None => {
                    tracing::warn!(
                        "BroadcastLogLayer not installed \
                         — /api/v1/logs and /api/v1/logs/history will return 503"
                    );
                    (None, None)
                }
            };

            // Register API config provider for dynamic server management
            let server_dir = Arc::new(ServerDir::open(&data_dir).map_err(|e| {
                PluginError::InitFailed(format!("Failed to open the servers directory: {e}"))
            })?);
            let provider_sender = Arc::new(tokio::sync::Mutex::new(None));

            let provider = ApiConfigProvider {
                dir: server_dir.clone(),
                sender: provider_sender.clone(),
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
                log_tx,
                log_history,
                server_dir,
                provider_sender,
                health_cache: Arc::new(HealthCache::new()),
                health_checker: Arc::new(HealthChecker::new()),
                recent_events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
                drain_store,
            });

            // Wire up EventBridge: proxy EventBus → broadcast::Sender<ApiEvent>
            let bridge = EventBridge::new(event_tx.clone(), ctx.player_registry_handle());
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::body::Body;
    use axum::http::{self, HeaderName, HeaderValue, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use infrarust_api::error::ServiceError;
    use infrarust_api::event::{BoxFuture, ListenerHandle};
    use infrarust_api::player::Player;
    use infrarust_api::services::ban_service::{BanEntry, BanTarget};
    use infrarust_api::services::config_service::{ServerConfig, ServerSource};
    use infrarust_api::services::load_balancer::{
        BackendState, BackendStatus, LbError, LoadBalancerService,
    };
    use infrarust_api::services::plugin_registry::{PluginDependencyInfo, PluginInfo};
    use infrarust_api::services::server_manager::{ServerState, StateChangeCallback};
    use infrarust_api::types::{PlayerId, ServerAddress, ServerId};
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::config::{ApiConfig, RateLimitConfig};
    use crate::rate_limit::RateLimiter;
    use crate::router::build_router;
    use crate::state::{ApiEvent, ApiState};

    // ── Mock PlayerRegistry ──

    struct MockPlayerRegistry {
        count: usize,
    }

    impl infrarust_api::services::player_registry::private::Sealed for MockPlayerRegistry {}

    impl infrarust_api::services::player_registry::PlayerRegistry for MockPlayerRegistry {
        fn get_player(&self, _username: &str) -> Option<Arc<dyn Player>> {
            None
        }
        fn get_player_by_uuid(&self, _uuid: &Uuid) -> Option<Arc<dyn Player>> {
            None
        }
        fn get_player_by_id(&self, _id: PlayerId) -> Option<Arc<dyn Player>> {
            None
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
            0
        }
    }

    // ── Mock BanService ──

    struct MockBanService;

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

    // ── Mock ServerManager ──

    struct MockServerManager;

    impl infrarust_api::services::server_manager::private::Sealed for MockServerManager {}

    impl infrarust_api::services::server_manager::ServerManager for MockServerManager {
        fn get_state(&self, _server: &ServerId) -> Option<ServerState> {
            None
        }
        fn start(&self, _server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
            Box::pin(async { Ok(()) })
        }
        fn stop(&self, _server: &ServerId) -> BoxFuture<'_, Result<(), ServiceError>> {
            Box::pin(async { Ok(()) })
        }
        fn on_state_change(&self, _callback: StateChangeCallback) -> ListenerHandle {
            ListenerHandle::new(0)
        }
        fn get_all_servers(&self) -> Vec<(ServerId, ServerState)> {
            vec![]
        }
    }

    // ── Mock ConfigService ──

    const MOCK_PROXY_CONFIG: &str = "\
# the proxy listens here
bind = \"0.0.0.0:25565\"
servers_dir = \"./servers\"

[web]
bind = \"127.0.0.1:8080\"
api_key = \"super-secret-key-value\"
";

    /// Where `--servers-dir` left the running proxy, which the stored document
    /// does not carry.
    const MOCK_OVERRIDDEN_SERVERS_DIR: &str = "/app/config/servers";

    /// Mirrors the real service: the stored document keeps its secrets, reads
    /// hand out a redacted copy, and writes restore what was not submitted.
    struct MockConfigService {
        server_count: usize,
        /// A server another provider also supplies, on top of the plugin's own.
        shadowed: Option<String>,
        proxy_document: std::sync::Mutex<String>,
    }

    impl MockConfigService {
        fn new(server_count: usize) -> Self {
            Self {
                server_count,
                shadowed: None,
                proxy_document: std::sync::Mutex::new(MOCK_PROXY_CONFIG.to_string()),
            }
        }

        fn with_shadow(server_count: usize, id: &str) -> Self {
            Self {
                shadowed: Some(id.to_string()),
                ..Self::new(server_count)
            }
        }

        fn ids(&self) -> impl Iterator<Item = String> + '_ {
            (0..self.server_count)
                .map(|i| format!("server_{i}"))
                .chain(self.shadowed.clone())
        }
    }

    impl infrarust_api::services::config_service::private::Sealed for MockConfigService {}

    impl infrarust_api::services::config_service::ConfigService for MockConfigService {
        fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
            self.ids().find(|id| id == server.as_str()).map(|id| {
                ServerConfig::new(
                    ServerId::new(id),
                    None,
                    vec![],
                    vec![],
                    infrarust_api::services::config_service::ProxyMode::Passthrough,
                    vec![],
                    0,
                    None,
                    false,
                    false,
                )
            })
        }
        fn get_all_server_configs(&self) -> Vec<ServerConfig> {
            self.ids()
                .map(|id| {
                    ServerConfig::new(
                        ServerId::new(id),
                        None,
                        vec![],
                        vec![],
                        infrarust_api::services::config_service::ProxyMode::Passthrough,
                        vec![],
                        0,
                        None,
                        false,
                        false,
                    )
                })
                .collect()
        }
        fn get_server_document(&self, server: &ServerId) -> Option<String> {
            self.ids()
                .find(|id| id == server.as_str())
                .map(|id| format!("id = \"{id}\"\naddresses = [\"127.0.0.1:25565\"]\n"))
        }
        fn list_server_sources(&self) -> Vec<ServerSource> {
            let mut sources: Vec<ServerSource> = self
                .ids()
                .map(|id| ServerSource {
                    id,
                    provider_id: "file@server.toml".to_string(),
                    provider_type: "file".to_string(),
                    editable: false,
                })
                .collect();
            if let Some(id) = &self.shadowed {
                sources.push(ServerSource {
                    id: id.clone(),
                    provider_id: format!("plugin:admin_api:api@{id}"),
                    provider_type: "plugin:admin_api:api".to_string(),
                    editable: true,
                });
            }
            sources
        }
        fn get_effective_proxy_config_document(&self) -> String {
            self.get_proxy_config_document()
                .replace("./servers", MOCK_OVERRIDDEN_SERVERS_DIR)
        }
        fn get_proxy_config_document(&self) -> String {
            let stored = self.proxy_document.lock().unwrap();
            let mut document: toml_edit::DocumentMut = stored.parse().unwrap();
            infrarust_config::secrets::redact(
                &mut document,
                infrarust_config::secrets::PROXY_SECRETS,
            );
            document.to_string()
        }
        fn write_proxy_config_document(
            &self,
            toml: &str,
        ) -> Result<(), infrarust_api::services::config_service::ConfigWriteError> {
            use infrarust_api::services::config_service::ConfigWriteError;
            let mut document: toml_edit::DocumentMut = toml
                .parse()
                .map_err(|e: toml_edit::TomlError| ConfigWriteError::Parse(e.to_string()))?;
            let mut stored = self.proxy_document.lock().unwrap();
            let current: toml_edit::DocumentMut = stored.parse().unwrap();
            infrarust_config::secrets::reinject(
                &mut document,
                &current,
                infrarust_config::secrets::PROXY_SECRETS,
            );
            let text = document.to_string();
            toml::from_str::<infrarust_config::ProxyConfig>(&text)
                .map_err(|e| ConfigWriteError::Parse(e.to_string()))?;
            *stored = text;
            Ok(())
        }
        fn get_value(&self, _key: &str) -> Option<String> {
            None
        }
    }

    // ── Mock LoadBalancerService ──

    /// Knows one backend per `MockConfigService` server, plus a drain flag so
    /// the mutation endpoints can be observed.
    struct MockLoadBalancerService {
        server_count: usize,
        drained: std::sync::Mutex<std::collections::HashSet<String>>,
    }

    impl MockLoadBalancerService {
        fn address_of(id: &str) -> Option<ServerAddress> {
            id.strip_prefix("server_")
                .and_then(|n| n.parse::<u8>().ok())
                .map(|n| ServerAddress {
                    host: format!("10.0.0.{n}"),
                    port: 25565,
                })
        }

        fn known(&self, server: &ServerId) -> Option<ServerAddress> {
            let address = Self::address_of(server.as_str())?;
            (0..self.server_count)
                .any(|i| format!("server_{i}") == server.as_str())
                .then_some(address)
        }

        fn ensure_backend(&self, server: &ServerId, addr: &ServerAddress) -> Result<(), LbError> {
            let known = self
                .known(server)
                .ok_or_else(|| LbError::UnknownServer(server.clone()))?;
            if known != *addr {
                return Err(LbError::UnknownAddress {
                    server: server.clone(),
                    address: addr.clone(),
                });
            }
            Ok(())
        }
    }

    impl infrarust_api::services::load_balancer::private::Sealed for MockLoadBalancerService {}

    impl LoadBalancerService for MockLoadBalancerService {
        fn strategy(&self, server: &ServerId) -> Option<String> {
            self.known(server).map(|_| "least_conn".to_string())
        }

        fn backends(&self, server: &ServerId) -> Vec<BackendStatus> {
            let Some(address) = self.known(server) else {
                return vec![];
            };
            let drained = self.drained.lock().unwrap().contains(&address.to_string());
            vec![BackendStatus {
                address,
                weight: 2,
                effective_weight: 1,
                state: if drained {
                    BackendState::Draining
                } else {
                    BackendState::Healthy
                },
                active_connections: 4,
                healthy_since_secs: Some(30),
                ejections: 1,
                last_failure_secs_ago: Some(90),
            }]
        }

        fn set_drained(
            &self,
            server: &ServerId,
            addr: &ServerAddress,
            drained: bool,
        ) -> Result<(), LbError> {
            self.ensure_backend(server, addr)?;
            let mut set = self.drained.lock().unwrap();
            if drained {
                set.insert(addr.to_string());
            } else {
                set.remove(&addr.to_string());
            }
            Ok(())
        }

        fn reset_backend(&self, server: &ServerId, addr: &ServerAddress) -> Result<(), LbError> {
            self.ensure_backend(server, addr)
        }
    }

    // ── Mock PluginRegistry ──

    struct MockPluginRegistry;

    impl infrarust_api::services::plugin_registry::private::Sealed for MockPluginRegistry {}

    impl infrarust_api::services::plugin_registry::PluginRegistry for MockPluginRegistry {
        fn list_plugin_info(&self) -> Vec<PluginInfo> {
            vec![PluginInfo {
                id: "admin_api".to_string(),
                name: "Admin API".to_string(),
                version: "0.1.0".to_string(),
                authors: vec!["Test".to_string()],
                description: Some("Test plugin".to_string()),
                state: "enabled".to_string(),
                dependencies: vec![PluginDependencyInfo {
                    id: "core".to_string(),
                    optional: false,
                }],
            }]
        }
        fn plugin_info(&self, id: &str) -> Option<PluginInfo> {
            self.list_plugin_info().into_iter().find(|p| p.id == id)
        }
    }

    // ── Helpers ──

    fn test_state() -> (tempfile::TempDir, Arc<ApiState>) {
        test_state_custom("test-key", 1000)
    }

    fn test_state_custom(
        api_key: &str,
        requests_per_minute: u64,
    ) -> (tempfile::TempDir, Arc<ApiState>) {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), api_key, requests_per_minute);
        (dir, state)
    }

    fn test_state_in(
        data_dir: &std::path::Path,
        api_key: &str,
        requests_per_minute: u64,
    ) -> Arc<ApiState> {
        test_state_with_config(
            data_dir,
            api_key,
            requests_per_minute,
            Arc::new(MockConfigService::new(2)),
        )
    }

    fn test_state_with_config(
        data_dir: &std::path::Path,
        api_key: &str,
        requests_per_minute: u64,
        config_service: Arc<MockConfigService>,
    ) -> Arc<ApiState> {
        let (event_tx, _) = broadcast::channel::<ApiEvent>(16);
        Arc::new(ApiState {
            player_registry: Arc::new(MockPlayerRegistry { count: 3 }),
            ban_service: Arc::new(MockBanService),
            server_manager: Arc::new(MockServerManager),
            config_service,
            load_balancer: Arc::new(MockLoadBalancerService {
                server_count: 2,
                drained: std::sync::Mutex::new(std::collections::HashSet::new()),
            }),
            plugin_registry: Arc::new(MockPluginRegistry),
            config: ApiConfig {
                bind: "127.0.0.1:0".into(),
                api_key: api_key.into(),
                cors_origins: vec![],
                rate_limit: RateLimitConfig::default(),
            },
            start_time: Instant::now(),
            proxy_version: "2.0.0-test".into(),
            rate_limiter: RateLimiter::new(requests_per_minute),
            event_tx,
            shutdown: CancellationToken::new(),
            proxy_shutdown: CancellationToken::new(),
            log_tx: None,
            log_history: None,
            server_dir: Arc::new(crate::server_dir::ServerDir::open(data_dir).unwrap()),
            provider_sender: Arc::new(tokio::sync::Mutex::new(None)),
            health_cache: Arc::new(crate::health_cache::HealthCache::new()),
            health_checker: Arc::new(crate::health_checker::HealthChecker::new()),
            recent_events: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            drain_store: Arc::new(crate::drain_store::DrainStore::open(data_dir)),
        })
    }

    fn auth_header() -> (HeaderName, HeaderValue) {
        (
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-key"),
        )
    }

    async fn response_body(response: http::Response<Body>) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn auth_get(uri: &str) -> (StatusCode, serde_json::Value) {
        let (_dir, state) = test_state();
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .uri(uri)
            .header(name, value)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn auth_post(uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let (_dir, state) = test_state();
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::POST)
            .uri(uri)
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn post_json(
        state: Arc<ApiState>,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::POST)
            .uri(uri)
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn put_json(
        state: Arc<ApiState>,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::PUT)
            .uri(uri)
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    async fn auth_delete(uri: &str) -> (StatusCode, serde_json::Value) {
        let (_dir, state) = test_state();
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::DELETE)
            .uri(uri)
            .header(name, value)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    // ── Health & Auth ──

    #[tokio::test]
    async fn test_health_returns_200_without_auth() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response_body(response).await;
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn test_proxy_status_returns_200_with_auth() {
        let (status, _) = auth_get("/api/v1/proxy").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_proxy_status_returns_401_without_auth_with_error_body() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/proxy")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = response_body(response).await;
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing")
        );
    }

    #[tokio::test]
    async fn test_proxy_status_returns_401_with_bad_key() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/proxy")
            .header(header::AUTHORIZATION, "Bearer wrong-key")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = response_body(response).await;
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("invalid")
        );
    }

    #[tokio::test]
    async fn test_empty_configured_key_rejects_empty_token() {
        let (_dir, state) = test_state_custom("", 1000);
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/proxy")
            .header(header::AUTHORIZATION, "Bearer ")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_failed_auth_is_rate_limited() {
        let (_dir, state) = test_state_custom("test-key", 2);
        let app = build_router(state, true);

        for _ in 0..2 {
            let request = Request::builder()
                .uri("/api/v1/proxy")
                .header(header::AUTHORIZATION, "Bearer wrong-key")
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let request = Request::builder()
            .uri("/api/v1/proxy")
            .header(header::AUTHORIZATION, "Bearer wrong-key")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_sse_routes_are_rate_limited() {
        let (_dir, state) = test_state_custom("test-key", 1);
        let app = build_router(state, true);

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);

        let second = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_proxy_status_returns_401_with_non_bearer() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/proxy")
            .header(header::AUTHORIZATION, "Basic dXNlcjpwYXNz")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_proxy_status_contains_expected_fields() {
        let (status, body) = auth_get("/api/v1/proxy").await;
        assert_eq!(status, StatusCode::OK);

        let data = &body["data"];
        assert_eq!(data["version"], "2.0.0-test");
        assert_eq!(data["players_online"], 3);
        assert_eq!(data["servers_count"], 2);
        assert!(data["uptime_seconds"].is_u64());
        assert!(data["uptime_human"].is_string());
        assert!(data["bind_address"].is_string());
        assert!(data["features"].is_array());
    }

    #[tokio::test]
    async fn test_unknown_route_returns_404() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/unknown")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // ── Players ──

    #[tokio::test]
    async fn test_players_list_returns_200_with_empty_list() {
        let (status, body) = auth_get("/api/v1/players").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].is_array());
        assert_eq!(body["data"].as_array().unwrap().len(), 0);
        assert_eq!(body["meta"]["total"], 0);
        assert_eq!(body["meta"]["page"], 1);
        assert_eq!(body["meta"]["per_page"], 20);
        assert_eq!(body["meta"]["total_pages"], 1);
    }

    #[tokio::test]
    async fn test_players_count_returns_200() {
        let (status, body) = auth_get("/api/v1/players/count").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["total"], 0);
        assert!(body["data"]["by_server"].is_object());
        assert!(body["data"]["by_mode"].is_object());
    }

    #[tokio::test]
    async fn test_players_get_returns_404_for_unknown() {
        let (status, body) = auth_get("/api/v1/players/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    // ── Bans ──

    #[tokio::test]
    async fn test_bans_list_returns_200_with_empty_list() {
        let (status, body) = auth_get("/api/v1/bans").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].is_array());
        assert_eq!(body["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn test_bans_check_returns_not_banned() {
        let (status, body) = auth_get("/api/v1/bans/check/username/Steve").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["banned"], false);
        assert!(body["data"]["ban"].is_null());
    }

    #[tokio::test]
    async fn test_bans_check_invalid_target_type() {
        let (status, body) = auth_get("/api/v1/bans/check/email/test@test.com").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
    }

    // ── Servers ──

    #[tokio::test]
    async fn test_servers_list_returns_200() {
        let (status, body) = auth_get("/api/v1/servers").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].is_array());
        assert_eq!(body["data"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_servers_get_returns_404_for_unknown() {
        let (status, body) = auth_get("/api/v1/servers/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn test_servers_list_reports_source_and_editable() {
        let (status, body) = auth_get("/api/v1/servers").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"][0]["source"], "file");
        assert_eq!(body["data"][0]["editable"], false);
    }

    #[tokio::test]
    async fn test_servers_raw_returns_toml() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .uri("/api/v1/servers/server_0/raw")
            .header(name, value)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("id = \"server_0\""));
    }

    #[tokio::test]
    async fn test_servers_config_returns_the_whole_config_as_json() {
        let (status, body) = auth_get("/api/v1/servers/server_0/config").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["id"], "server_0");
        assert_eq!(body["data"]["addresses"][0], "127.0.0.1:25565");
        assert_eq!(body["data"]["proxy_mode"], "passthrough");
        assert_eq!(body["data"]["balance"], "first_available");
    }

    #[tokio::test]
    async fn test_servers_config_unknown_is_not_found() {
        let (status, _) = auth_get("/api/v1/servers/nope/config").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_servers_write_to_file_provider_is_forbidden() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::PUT)
            .uri("/api/v1/servers/server_0")
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "id": "server_0",
                    "domains": ["a.example.com"],
                    "addresses": ["127.0.0.1:25565"],
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_servers_validate_accepts_valid_json() {
        let (status, body) = auth_post(
            "/api/v1/servers/validate",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565"],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["valid"], true);
        assert!(body["data"]["errors"].as_array().unwrap().is_empty());
    }

    /// The dashboard validates the document it was handed, credential and all,
    /// so a redacted one has to be checkable as it stands.
    #[tokio::test]
    async fn test_servers_validate_accepts_a_redacted_document() {
        let (status, body) = auth_post(
            "/api/v1/servers/validate",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565"],
                "server_manager": {
                    "type": "pterodactyl",
                    "api_url": "https://panel.example.com",
                    "api_key": infrarust_config::secrets::REDACTED,
                    "server_id": "abc",
                },
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["valid"], true, "{body}");
    }

    #[tokio::test]
    async fn test_servers_validate_reports_errors() {
        let (status, body) = auth_post(
            "/api/v1/servers/validate",
            serde_json::json!({
                "id": "lobby",
                "domains": [],
                "addresses": ["127.0.0.1:25565"],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["valid"], false);
        assert!(!body["data"]["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_servers_validate_reports_warnings() {
        let (status, body) = auth_post(
            "/api/v1/servers/validate",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["127.0.0.1:25565", "127.0.0.1:25566"],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["valid"], true);
        assert!(!body["data"]["warnings"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_servers_create_persists_full_document() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let app = build_router(state.clone(), true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::POST)
            .uri("/api/v1/servers")
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "id": "survival",
                    "domains": ["mc.example.com"],
                    "addresses": [
                        "10.0.0.1:25565",
                        { "address": "10.0.0.2:25565", "weight": 3 },
                    ],
                    "balance": "least_conn",
                    "slow_start": "45s",
                    "motd": { "online": { "text": "Welcome" } },
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let stored = std::fs::read_to_string(dir.path().join("servers/survival.toml")).unwrap();
        let config = crate::server_dir::parse_document("survival", &stored).unwrap();
        assert_eq!(config.balance, infrarust_config::BalanceStrategy::LeastConn);
        assert_eq!(config.addresses[1].weight, 3);
        assert_eq!(config.motd.online.as_ref().unwrap().text, "Welcome");
        assert!(state.server_dir.owns("survival"));
    }

    const MANAGED_SERVER: &str = "\
id = \"survival\"
domains = [\"mc.example.com\"]
addresses = [\"10.0.0.1:25565\"]

[server_manager]
type = \"pterodactyl\"
api_url = \"https://panel.example.com\"
api_key = \"ptlc_live_xxx\"
server_id = \"abc\"
";

    /// Returns a state whose directory already holds `survival.toml`, a server
    /// with a panel credential in it.
    fn state_with_a_managed_server(dir: &tempfile::TempDir) -> Arc<ApiState> {
        std::fs::create_dir_all(dir.path().join("servers")).unwrap();
        std::fs::write(dir.path().join("servers/survival.toml"), MANAGED_SERVER).unwrap();
        test_state_in(dir.path(), "test-key", 1000)
    }

    #[tokio::test]
    async fn test_servers_raw_hides_the_manager_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_a_managed_server(&dir);

        let (status, _, text) = auth_text(
            &state,
            http::Method::GET,
            "/api/v1/servers/survival/raw",
            None,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(!text.contains("ptlc_live_xxx"));
        assert!(text.contains(infrarust_config::secrets::REDACTED));
        assert!(text.contains("https://panel.example.com"));
    }

    #[tokio::test]
    async fn test_servers_config_hides_the_manager_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_a_managed_server(&dir);

        let (status, _, body) = auth_text(
            &state,
            http::Method::GET,
            "/api/v1/servers/survival/config",
            None,
        )
        .await;
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["server_manager"]["api_key"],
            infrarust_config::secrets::REDACTED
        );
    }

    /// A client edits a document it was never shown the credential of, so
    /// saving it back must restore the stored one rather than store the
    /// placeholder.
    #[tokio::test]
    async fn test_servers_raw_round_trip_keeps_the_manager_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_a_managed_server(&dir);
        let (_, _, document) = auth_text(
            &state,
            http::Method::GET,
            "/api/v1/servers/survival/raw",
            None,
        )
        .await;
        let edited = document.replace("mc.example.com", "play.example.com");

        let (status, _, _) = auth_text(
            &state,
            http::Method::PUT,
            "/api/v1/servers/survival/raw",
            Some(&edited),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let stored = std::fs::read_to_string(dir.path().join("servers/survival.toml")).unwrap();
        assert!(stored.contains("ptlc_live_xxx"));
        assert!(stored.contains("play.example.com"));
        assert!(!stored.contains(infrarust_config::secrets::REDACTED));
    }

    #[tokio::test]
    async fn test_servers_update_keeps_the_manager_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_with_a_managed_server(&dir);
        let (_, _, body) = auth_text(
            &state,
            http::Method::GET,
            "/api/v1/servers/survival/config",
            None,
        )
        .await;
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();

        let app = build_router(state.clone(), true);
        let (name, value) = auth_header();
        let request = Request::builder()
            .method(http::Method::PUT)
            .uri("/api/v1/servers/survival")
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body["data"].to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let stored = std::fs::read_to_string(dir.path().join("servers/survival.toml")).unwrap();
        assert!(stored.contains("ptlc_live_xxx"));
    }

    #[tokio::test]
    async fn test_servers_create_rejects_a_secret_it_cannot_restore() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, body) = post_json(
            state,
            "/api/v1/servers",
            serde_json::json!({
                "id": "survival",
                "domains": ["mc.example.com"],
                "addresses": ["10.0.0.1:25565"],
                "server_manager": {
                    "type": "pterodactyl",
                    "api_url": "https://panel.example.com",
                    "api_key": infrarust_config::secrets::REDACTED,
                    "server_id": "abc",
                },
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
        assert!(!dir.path().join("servers/survival.toml").exists());
    }

    /// A file whose config renames itself through `name` is known to the proxy
    /// under that name, not under its stem. Creating a server that lands on the
    /// same file must not quietly destroy it.
    #[tokio::test]
    async fn test_servers_create_refuses_a_taken_file_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("servers")).unwrap();
        let path = dir.path().join("servers/foo.toml");
        std::fs::write(
            &path,
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .unwrap();

        let state = test_state_in(dir.path(), "test-key", 1000);
        let (status, _) = post_json(
            state,
            "/api/v1/servers",
            serde_json::json!({
                "id": "foo",
                "domains": ["foo.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(std::fs::read_to_string(&path).unwrap().contains("bar"));
    }

    /// `settle_id` leaves `id` unset when the config names itself, so the
    /// stored copy must still match what the watcher reads back.
    #[tokio::test]
    async fn test_servers_created_by_name_do_not_echo_back_as_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let sender = crate::server_dir::test_support::RecordingSender::default();
        *state.provider_sender.lock().await = Some(Box::new(sender.clone()));

        let (status, _) = post_json(
            state.clone(),
            "/api/v1/servers",
            serde_json::json!({
                "name": "survival",
                "domains": ["mc.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let summary = crate::server_dir::reload(&state.server_dir, &state.provider_sender)
            .await
            .unwrap();
        assert_eq!((summary.added, summary.updated, summary.removed), (0, 0, 0));
        assert_eq!(sender.events(), vec!["added:survival".to_string()]);
    }

    /// The router must learn about a write before another writer can touch the
    /// directory, or two concurrent updates can reach it in the wrong order.
    #[tokio::test]
    async fn test_a_write_reaches_the_proxy_before_the_directory_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let (sender, announcements) = crate::server_dir::test_support::DocumentDuringSend::new(
            state.server_dir.clone(),
            Duration::from_millis(100),
        );
        *state.provider_sender.lock().await = Some(Box::new(sender));

        let (status, _) = post_json(
            state.clone(),
            "/api/v1/servers",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let writers: Vec<_> = (0..4)
            .map(|i| {
                let state = state.clone();
                tokio::spawn(async move {
                    put_json(
                        state,
                        "/api/v1/servers/lobby",
                        serde_json::json!({
                            "id": "lobby",
                            "domains": [format!("lobby{i}.example.com")],
                            "addresses": ["10.0.0.1:25565"],
                        }),
                    )
                    .await
                    .0
                })
            })
            .collect();
        for writer in writers {
            assert_eq!(writer.await.unwrap(), StatusCode::OK);
        }

        let announcements = announcements.lock().unwrap().clone();
        assert_eq!(announcements.len(), 5);
        for (announced, stored) in announcements {
            assert_eq!(
                stored.as_deref(),
                Some(announced.as_str()),
                "the proxy was told about a document the directory no longer held"
            );
        }
    }

    #[tokio::test]
    async fn test_writes_and_the_watcher_do_not_deadlock() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let sender = crate::server_dir::test_support::SlowSender::new(Duration::from_millis(100));
        *state.provider_sender.lock().await = Some(Box::new(sender));

        let shutdown = CancellationToken::new();
        tokio::spawn(crate::server_dir::watch(
            state.server_dir.clone(),
            state.provider_sender.clone(),
            shutdown.clone(),
        ));

        let statuses = tokio::time::timeout(Duration::from_secs(20), async {
            let writers: Vec<_> = (0..6)
                .map(|i| {
                    let state = state.clone();
                    tokio::spawn(async move {
                        post_json(
                            state,
                            "/api/v1/servers",
                            serde_json::json!({
                                "id": format!("lobby-{i}"),
                                "domains": [format!("lobby{i}.example.com")],
                                "addresses": ["10.0.0.1:25565"],
                            }),
                        )
                        .await
                        .0
                    })
                })
                .collect();

            let mut statuses = Vec::new();
            for writer in writers {
                statuses.push(writer.await.unwrap());
            }
            statuses
        })
        .await
        .expect("the writers and the directory watcher deadlocked");
        shutdown.cancel();

        assert_eq!(statuses.len(), 6);
        assert!(statuses.iter().all(|status| *status == StatusCode::CREATED));
    }

    /// A server the proxy also gets from somewhere else cannot be edited here:
    /// the write would land on a copy the router may not be the one routing.
    #[tokio::test]
    async fn test_servers_supplied_twice_are_not_editable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("servers")).unwrap();
        std::fs::write(
            dir.path().join("servers/lobby.toml"),
            "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n",
        )
        .unwrap();

        let state = test_state_with_config(
            dir.path(),
            "test-key",
            1000,
            Arc::new(MockConfigService::with_shadow(2, "lobby")),
        );

        let (status, _) = put_json(
            state.clone(),
            "/api/v1/servers/lobby",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["10.0.0.2:25565"],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        let app = build_router(state, true);
        let (name, value) = auth_header();
        let request = Request::builder()
            .uri("/api/v1/servers/lobby")
            .header(name, value)
            .body(Body::empty())
            .unwrap();
        let detail = response_body(app.oneshot(request).await.unwrap()).await;
        assert_eq!(detail["data"]["editable"], false);
    }

    /// The file exists, but under a stem that is not the server id, so writing
    /// `<id>.toml` would leave two documents claiming one server.
    #[tokio::test]
    async fn test_servers_update_of_a_shadowed_document_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("servers")).unwrap();
        std::fs::write(
            dir.path().join("servers/foo.toml"),
            "name = \"bar\"\ndomains = [\"bar.example.com\"]\naddresses = [\"10.0.0.9:25565\"]\n",
        )
        .unwrap();

        let state = test_state_in(dir.path(), "test-key", 1000);
        let (status, _) = put_json(
            state,
            "/api/v1/servers/bar",
            serde_json::json!({
                "name": "bar",
                "domains": ["bar.example.com"],
                "addresses": ["10.0.0.8:25565"],
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!dir.path().join("servers/bar.toml").exists());
    }

    #[tokio::test]
    async fn test_servers_create_rejects_traversal_id() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let app = build_router(state, true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(http::Method::POST)
            .uri("/api/v1/servers")
            .header(name, value)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "id": "../escaped",
                    "domains": ["mc.example.com"],
                    "addresses": ["10.0.0.1:25565"],
                })
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!dir.path().join("servers/__escaped.toml").exists());
    }

    // ── Plugins ──

    #[tokio::test]
    async fn test_plugins_list_returns_200() {
        let (status, body) = auth_get("/api/v1/plugins").await;
        assert_eq!(status, StatusCode::OK);
        let plugins = body["data"].as_array().unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0]["id"], "admin_api");
        assert_eq!(plugins[0]["state"], "enabled");
    }

    #[tokio::test]
    async fn test_plugins_get_returns_200() {
        let (status, body) = auth_get("/api/v1/plugins/admin_api").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["id"], "admin_api");
        assert_eq!(body["data"]["version"], "0.1.0");
    }

    #[tokio::test]
    async fn test_plugins_get_returns_404_for_unknown() {
        let (status, body) = auth_get("/api/v1/plugins/nonexistent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    // ── Stats ──

    #[tokio::test]
    async fn test_stats_overview_returns_200() {
        let (status, body) = auth_get("/api/v1/stats").await;
        assert_eq!(status, StatusCode::OK);
        let data = &body["data"];
        assert_eq!(data["players_online"], 0);
        assert_eq!(data["servers_total"], 2);
        assert!(data["uptime_seconds"].is_u64());
        assert!(data["players_by_server"].is_object());
        assert!(data["servers_by_state"].is_object());
    }

    // ── Config ──

    #[tokio::test]
    async fn test_config_providers_returns_200() {
        let (status, body) = auth_get("/api/v1/config/providers").await;
        assert_eq!(status, StatusCode::OK);
        let providers = body["data"].as_array().unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["provider_type"], "file");
        assert_eq!(providers[0]["configs_count"], 2);
    }

    // ── Auth on all endpoints ──

    #[tokio::test]
    async fn test_new_endpoints_require_auth() {
        let endpoints = [
            "/api/v1/players",
            "/api/v1/players/count",
            "/api/v1/bans",
            "/api/v1/servers",
            "/api/v1/plugins",
            "/api/v1/stats",
            "/api/v1/config/providers",
            "/api/v1/health/backends",
            "/api/v1/servers/server_0/backends",
        ];

        for uri in endpoints {
            let (_dir, state) = test_state();
            let app = build_router(state, true);

            let request = Request::builder().uri(uri).body(Body::empty()).unwrap();

            let response = app.oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "Expected 401 for {uri} without auth"
            );
        }
    }

    // ── Player Mutations ──

    #[tokio::test]
    async fn test_kick_player_not_found() {
        let (status, body) = auth_post(
            "/api/v1/players/unknown/kick",
            serde_json::json!({"reason": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn test_send_player_not_found() {
        let (status, _) = auth_post(
            "/api/v1/players/unknown/send",
            serde_json::json!({"server": "lobby"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_message_player_not_found() {
        let (status, _) = auth_post(
            "/api/v1/players/unknown/message",
            serde_json::json!({"text": "hello"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_broadcast_returns_200() {
        let (status, body) = auth_post(
            "/api/v1/players/broadcast",
            serde_json::json!({"text": "hello everyone"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["success"], true);
        assert!(
            body["data"]["message"]
                .as_str()
                .unwrap()
                .contains("Broadcast")
        );
    }

    #[tokio::test]
    async fn test_broadcast_text_too_long_returns_400() {
        let (status, body) = auth_post(
            "/api/v1/players/broadcast",
            serde_json::json!({"text": "x".repeat(257)}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn test_message_text_too_long_returns_400() {
        let (status, body) = auth_post(
            "/api/v1/players/unknown/message",
            serde_json::json!({"text": "x".repeat(257)}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
    }

    // ── Ban Mutations ──

    #[tokio::test]
    async fn test_create_ban_returns_201() {
        let (status, body) = auth_post(
            "/api/v1/bans",
            serde_json::json!({
                "target": {"type": "username", "value": "griefer"},
                "reason": "griefing"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["data"]["success"], true);
    }

    #[tokio::test]
    async fn test_create_ban_invalid_ip() {
        let (status, body) = auth_post(
            "/api/v1/bans",
            serde_json::json!({
                "target": {"type": "ip", "value": "not-an-ip"}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "BAD_REQUEST");
    }

    #[tokio::test]
    async fn test_delete_ban_not_found() {
        let (status, body) = auth_delete("/api/v1/bans/username/nobody").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    // ── Server Mutations ──

    #[tokio::test]
    async fn test_server_start_not_found() {
        let (status, body) =
            auth_post("/api/v1/servers/nonexistent/start", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn test_server_stop_not_found() {
        let (status, body) =
            auth_post("/api/v1/servers/nonexistent/stop", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    // ── Config Mutations ──

    /// A reload the proxy never heard about must not read as a successful one.
    #[tokio::test]
    async fn test_config_reload_without_a_provider_is_unavailable() {
        let (status, body) = auth_post("/api/v1/config/reload", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
    }

    #[tokio::test]
    async fn test_config_reload_rescans_and_reports_what_it_found() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let sender = crate::server_dir::test_support::RecordingSender::default();
        *state.provider_sender.lock().await = Some(Box::new(sender.clone()));

        std::fs::write(
            dir.path().join("servers/lobby.toml"),
            "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n",
        )
        .unwrap();

        let (status, body) = post_json(state, "/api/v1/config/reload", serde_json::json!({})).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["details"]["added"], 1);
        assert_eq!(sender.events(), vec!["added:lobby".to_string()]);
    }

    // ── Plugin Mutations ──

    #[tokio::test]
    async fn test_plugin_disable_returns_503() {
        let (status, body) =
            auth_post("/api/v1/plugins/admin_api/disable", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
    }

    #[tokio::test]
    async fn test_plugin_enable_returns_503() {
        let (status, body) =
            auth_post("/api/v1/plugins/admin_api/enable", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
    }

    // ── Proxy Mutations ──

    #[tokio::test]
    async fn test_proxy_shutdown_returns_200() {
        let (status, body) = auth_post("/api/v1/proxy/shutdown", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["success"], true);
    }

    #[tokio::test]
    async fn test_proxy_gc_returns_200() {
        let (status, body) = auth_post("/api/v1/proxy/gc", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["success"], true);
    }

    // ── Mutation Auth ──

    #[tokio::test]
    async fn test_mutation_endpoints_require_auth() {
        let endpoints: Vec<(&str, &str)> = vec![
            ("POST", "/api/v1/players/broadcast"),
            ("POST", "/api/v1/players/test/kick"),
            ("POST", "/api/v1/bans"),
            ("DELETE", "/api/v1/bans/username/test"),
            ("POST", "/api/v1/servers/test/start"),
            (
                "POST",
                "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
            ),
            (
                "POST",
                "/api/v1/servers/server_0/backends/10.0.0.0:25565/enable",
            ),
            (
                "POST",
                "/api/v1/servers/server_0/backends/10.0.0.0:25565/reset",
            ),
            ("POST", "/api/v1/config/reload"),
            ("POST", "/api/v1/proxy/shutdown"),
            ("POST", "/api/v1/proxy/gc"),
        ];

        for (method, uri) in endpoints {
            let (_dir, state) = test_state();
            let app = build_router(state, true);
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "Expected 401 for {method} {uri}"
            );
        }
    }

    // ── Backends ──

    async fn auth_send(
        state: &Arc<ApiState>,
        method: http::Method,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let app = build_router(state.clone(), true);
        let (name, value) = auth_header();

        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(name, value)
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, body)
    }

    #[tokio::test]
    async fn test_backends_list_reports_strategy_and_addresses() {
        let (status, body) = auth_get("/api/v1/servers/server_0/backends").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["strategy"], "least_conn");

        let backend = &body["data"]["backends"][0];
        assert_eq!(backend["address"], "10.0.0.0:25565");
        assert_eq!(backend["state"], "healthy");
        assert_eq!(backend["weight"], 2);
        assert_eq!(backend["effective_weight"], 1);
        assert_eq!(backend["active_connections"], 4);
        assert_eq!(backend["ejections"], 1);
        assert_eq!(backend["healthy_since_secs"], 30);
        assert_eq!(backend["last_failure_secs_ago"], 90);
    }

    #[tokio::test]
    async fn test_backends_list_unknown_server_returns_404() {
        let (status, _) = auth_get("/api/v1/servers/ghost/backends").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_proxy_wide_backends_are_grouped_by_server() {
        let (status, body) = auth_get("/api/v1/health/backends").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["server_0"]["backends"][0]["address"],
            "10.0.0.0:25565"
        );
        assert_eq!(body["data"]["server_1"]["strategy"], "least_conn");
    }

    #[tokio::test]
    async fn test_drain_then_enable_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0%3A25565/drain",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, body) = auth_send(
            &state,
            http::Method::GET,
            "/api/v1/servers/server_0/backends",
        )
        .await;
        assert_eq!(body["data"]["backends"][0]["state"], "draining");

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/enable",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, body) = auth_send(
            &state,
            http::Method::GET,
            "/api/v1/servers/server_0/backends",
        )
        .await;
        assert_eq!(body["data"]["backends"][0]["state"], "healthy");
    }

    #[tokio::test]
    async fn test_drain_survives_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
        )
        .await;

        assert_eq!(
            crate::drain_store::DrainStore::open(dir.path()).entries(),
            vec![(
                "server_0".to_string(),
                crate::util::parse_address("10.0.0.0:25565").unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn test_deleting_a_server_forgets_its_drained_backends() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, _) = post_json(
            state.clone(),
            "/api/v1/servers",
            serde_json::json!({
                "id": "lobby",
                "domains": ["lobby.example.com"],
                "addresses": ["10.0.0.1:25565"],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        state
            .drain_store
            .set(
                "lobby",
                &crate::util::parse_address("10.0.0.1:25565").unwrap(),
                true,
            )
            .await
            .unwrap();

        let (status, _) = auth_send(&state, http::Method::DELETE, "/api/v1/servers/lobby").await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            crate::drain_store::DrainStore::open(dir.path())
                .entries()
                .is_empty()
        );
    }

    /// The store must be keyed on the address, not on however the caller
    /// happened to spell it in the URL.
    #[tokio::test]
    async fn test_enabling_a_backend_clears_it_whatever_the_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565%20/enable",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        assert!(
            crate::drain_store::DrainStore::open(dir.path())
                .entries()
                .is_empty(),
            "a stale entry would re-drain the backend on the next start"
        );
    }

    #[tokio::test]
    async fn test_reset_backend_leaves_the_drain_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/drain",
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.0.0.0:25565/reset",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["success"], true);

        let (_, body) = auth_send(
            &state,
            http::Method::GET,
            "/api/v1/servers/server_0/backends",
        )
        .await;
        assert_eq!(body["data"]["backends"][0]["state"], "draining");
    }

    #[tokio::test]
    async fn test_backend_mutation_rejects_unknown_address() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, body) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/10.9.9.9:25565/drain",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn test_backend_mutation_rejects_a_malformed_address() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);

        let (status, _) = auth_send(
            &state,
            http::Method::POST,
            "/api/v1/servers/server_0/backends/not-an-address/drain",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ── Global proxy config ──

    async fn auth_text(
        state: &Arc<ApiState>,
        method: http::Method,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Option<HeaderValue>, String) {
        let app = build_router(state.clone(), true);
        let (name, value) = auth_header();

        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(name, value);
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "text/plain");
        }
        let request = builder
            .body(body.map_or_else(Body::empty, |text| Body::from(text.to_string())))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            content_type,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    fn config_state() -> (Arc<ApiState>, Arc<MockConfigService>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config_service = Arc::new(MockConfigService::new(2));
        let state = test_state_with_config(dir.path(), "test-key", 1000, config_service.clone());
        (state, config_service, dir)
    }

    #[tokio::test]
    async fn test_config_proxy_json_fills_defaults_and_hides_the_api_key() {
        let (status, body) = auth_get("/api/v1/config/proxy").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["bind"], "0.0.0.0:25565");
        assert_eq!(body["data"]["web"]["bind"], "127.0.0.1:8080");
        assert_eq!(
            body["data"]["web"]["api_key"],
            infrarust_config::secrets::REDACTED
        );
        assert_eq!(body["data"]["connect_timeout"], "5s");
        assert!(!body.to_string().contains("super-secret-key-value"));
    }

    /// The summary this feeds is where an operator reads which directory and
    /// which address the proxy is really on, overrides included.
    #[tokio::test]
    async fn test_config_proxy_json_reports_the_running_config() {
        let (status, body) = auth_get("/api/v1/config/proxy").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["servers_dir"], MOCK_OVERRIDDEN_SERVERS_DIR);
    }

    #[tokio::test]
    async fn test_config_proxy_raw_is_the_file_with_the_api_key_hidden() {
        let (state, _config, _dir) = config_state();
        let (status, content_type, text) =
            auth_text(&state, http::Method::GET, "/api/v1/config/proxy/raw", None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.unwrap(), "text/plain; charset=utf-8");
        assert!(!text.contains("super-secret-key-value"));
        assert!(text.contains(infrarust_config::secrets::REDACTED));
        assert!(text.contains("./servers"), "the file's own value");
        assert!(text.contains("# the proxy listens here"));
    }

    #[tokio::test]
    async fn test_config_proxy_raw_round_trip_keeps_the_api_key() {
        let (state, config, _dir) = config_state();
        let (_, _, document) =
            auth_text(&state, http::Method::GET, "/api/v1/config/proxy/raw", None).await;

        let (status, _, body) = auth_text(
            &state,
            http::Method::PUT,
            "/api/v1/config/proxy/raw",
            Some(&document),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["data"]["success"], true);
        assert_eq!(body["data"]["requires_restart"], true);

        let stored = config.proxy_document.lock().unwrap();
        assert!(stored.contains("api_key = \"super-secret-key-value\""));
        assert!(stored.contains("# the proxy listens here"));
    }

    #[tokio::test]
    async fn test_config_proxy_raw_rejects_a_malformed_document() {
        let (state, config, _dir) = config_state();
        let before = config.proxy_document.lock().unwrap().clone();

        let (status, _, _) = auth_text(
            &state,
            http::Method::PUT,
            "/api/v1/config/proxy/raw",
            Some("bind = [unclosed"),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(*config.proxy_document.lock().unwrap(), before);
    }

    #[tokio::test]
    async fn test_config_proxy_validate_accepts_a_valid_document() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state_in(dir.path(), "test-key", 1000);
        let document = format!(
            "bind = \"0.0.0.0:25565\"\nservers_dir = {:?}\n",
            dir.path().display()
        );

        let (status, _, body) = auth_text(
            &state,
            http::Method::POST,
            "/api/v1/config/proxy/validate",
            Some(&document),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["data"]["valid"], true);
    }

    /// The running proxy may have been started with `--servers-dir`, so the
    /// live config must not be reported invalid for naming another one.
    #[tokio::test]
    async fn test_config_proxy_validate_ignores_where_the_document_points() {
        let (state, _config, _dir) = config_state();
        let (_, _, document) =
            auth_text(&state, http::Method::GET, "/api/v1/config/proxy/raw", None).await;
        assert!(document.contains("./servers"));

        let (status, _, body) = auth_text(
            &state,
            http::Method::POST,
            "/api/v1/config/proxy/validate",
            Some(&document),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["data"]["valid"], true, "{body}");
    }

    #[tokio::test]
    async fn test_config_proxy_validate_reports_an_unknown_key() {
        let (state, _config, _dir) = config_state();

        let (status, _, body) = auth_text(
            &state,
            http::Method::POST,
            "/api/v1/config/proxy/validate",
            Some("bnid = \"0.0.0.0:25565\"\n"),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["data"]["valid"], false);
        assert!(!body["data"]["errors"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_config_proxy_requires_auth() {
        let (_dir, state) = test_state();
        let app = build_router(state, true);

        let request = Request::builder()
            .uri("/api/v1/config/proxy/raw")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
