pub mod api_provider;
pub mod auth;
pub mod config;
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
pub mod server_store;
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
use crate::health_cache::HealthCache;
use crate::health_checker::HealthChecker;
use crate::log_layer::LogBroadcast;
use crate::rate_limit::RateLimiter;

const EVENT_CHANNEL_CAPACITY: usize = 256;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
use crate::router::build_router;
use crate::server_store::ApiServerStore;
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
            let server_store = Arc::new(ApiServerStore::load(&data_dir));
            let provider_sender = Arc::new(tokio::sync::Mutex::new(None));

            let provider = ApiConfigProvider {
                store: server_store.clone(),
                sender: provider_sender.clone(),
            };
            ctx.register_config_provider(Box::new(provider));

            let start_time = Instant::now();

            let state = Arc::new(ApiState {
                player_registry: ctx.player_registry_handle(),
                ban_service: ctx.ban_service_handle(),
                server_manager: ctx.server_manager_handle(),
                config_service: ctx.config_service_handle(),
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
                server_store,
                provider_sender,
                health_cache: Arc::new(HealthCache::new()),
                health_checker: Arc::new(HealthChecker::new()),
                recent_events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
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
                                    Ok(event) => crate::state::push_recent_event(&recent, &event),
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
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
                if let Err(e) = axum::serve(listener, app)
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
    use infrarust_api::services::config_service::ServerConfig;
    use infrarust_api::services::plugin_registry::{PluginDependencyInfo, PluginInfo};
    use infrarust_api::services::server_manager::{ServerState, StateChangeCallback};
    use infrarust_api::types::{PlayerId, ServerId};
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

    struct MockConfigService {
        server_count: usize,
    }

    impl infrarust_api::services::config_service::private::Sealed for MockConfigService {}

    impl infrarust_api::services::config_service::ConfigService for MockConfigService {
        fn get_server_config(&self, _server: &ServerId) -> Option<ServerConfig> {
            None
        }
        fn get_all_server_configs(&self) -> Vec<ServerConfig> {
            (0..self.server_count)
                .map(|i| {
                    ServerConfig::new(
                        ServerId::new(format!("server_{i}")),
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
        fn get_value(&self, _key: &str) -> Option<String> {
            None
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

    fn test_state() -> Arc<ApiState> {
        let (event_tx, _) = broadcast::channel::<ApiEvent>(16);
        Arc::new(ApiState {
            player_registry: Arc::new(MockPlayerRegistry { count: 3 }),
            ban_service: Arc::new(MockBanService),
            server_manager: Arc::new(MockServerManager),
            config_service: Arc::new(MockConfigService { server_count: 2 }),
            plugin_registry: Arc::new(MockPluginRegistry),
            config: ApiConfig {
                bind: "127.0.0.1:0".into(),
                api_key: "test-key".into(),
                cors_origins: vec![],
                rate_limit: RateLimitConfig::default(),
            },
            start_time: Instant::now(),
            proxy_version: "2.0.0-test".into(),
            rate_limiter: RateLimiter::new(1000),
            event_tx,
            shutdown: CancellationToken::new(),
            proxy_shutdown: CancellationToken::new(),
            log_tx: None,
            log_history: None,
            server_store: Arc::new(crate::server_store::ApiServerStore::load(
                std::path::Path::new("/tmp/infrarust-test"),
            )),
            provider_sender: Arc::new(tokio::sync::Mutex::new(None)),
            health_cache: Arc::new(crate::health_cache::HealthCache::new()),
            health_checker: Arc::new(crate::health_checker::HealthChecker::new()),
            recent_events: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
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
        let state = test_state();
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
        let state = test_state();
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

    async fn auth_delete(uri: &str) -> (StatusCode, serde_json::Value) {
        let state = test_state();
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
        let state = test_state();
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
        let state = test_state();
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
        let state = test_state();
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
    async fn test_proxy_status_returns_401_with_non_bearer() {
        let state = test_state();
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
        let state = test_state();
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
        ];

        for uri in endpoints {
            let state = test_state();
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

    #[tokio::test]
    async fn test_config_reload_returns_503() {
        let (status, body) = auth_post("/api/v1/config/reload", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "SERVICE_UNAVAILABLE");
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
            ("POST", "/api/v1/config/reload"),
            ("POST", "/api/v1/proxy/shutdown"),
            ("POST", "/api/v1/proxy/gc"),
        ];

        for (method, uri) in endpoints {
            let app = build_router(test_state(), true);
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
}
