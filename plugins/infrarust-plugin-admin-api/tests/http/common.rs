use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::http::{self, HeaderName, HeaderValue, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use infrarust_api::services::config_service::{ConfigWriteError, ServerSource};
use infrarust_api::services::load_balancer::{BackendState, BackendStatus};
use infrarust_api::services::plugin_registry::{PluginDependencyInfo, PluginInfo};
use infrarust_api::test_util::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayer, MockPlayerRegistry,
    MockPluginRegistry, MockServerManager,
};
use infrarust_api::types::{ServerAddress, ServerId};
use infrarust_plugin_admin_api::config::{ApiConfig, RateLimitConfig};
use infrarust_plugin_admin_api::drain_store::DrainStore;
use infrarust_plugin_admin_api::health_cache::HealthCache;
use infrarust_plugin_admin_api::health_checker::HealthChecker;
use infrarust_plugin_admin_api::log_layer::LogBroadcast;
use infrarust_plugin_admin_api::rate_limit::RateLimiter;
use infrarust_plugin_admin_api::router::build_router;
use infrarust_plugin_admin_api::server_dir::ServerDir;
use infrarust_plugin_admin_api::state::{ApiEvent, ApiState};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

pub const SECRET_API_KEY: &str = "super-secret-key-value";

pub const PROXY_CONFIG: &str = "\
# the proxy listens here
bind = \"0.0.0.0:25565\"
servers_dir = \"./servers\"

[web]
bind = \"127.0.0.1:8080\"
api_key = \"super-secret-key-value\"
";

pub const OVERRIDDEN_SERVERS_DIR: &str = "/app/config/servers";

pub const MANAGED_SERVER: &str = "\
id = \"survival\"
domains = [\"mc.example.com\"]
addresses = [\"10.0.0.1:25565\"]

[server_manager]
type = \"pterodactyl\"
api_url = \"https://panel.example.com\"
api_key = \"ptlc_live_xxx\"
server_id = \"abc\"
";

pub fn config_service() -> MockConfigService {
    with_proxy_document(MockConfigService::new().with_servers(2))
}

pub fn shadowed_config_service(id: &str) -> MockConfigService {
    let source = ServerSource {
        id: id.to_owned(),
        provider_id: format!("plugin:admin_api:api@{id}"),
        provider_type: "plugin:admin_api:api".to_owned(),
        editable: true,
    };
    with_proxy_document(
        MockConfigService::new()
            .with_servers(2)
            .with_server(MockConfigService::server(id))
            .with_source(source),
    )
}

fn with_proxy_document(config: MockConfigService) -> MockConfigService {
    config
        .with_proxy_document(PROXY_CONFIG)
        .redacting(|stored| {
            let mut document: toml_edit::DocumentMut = stored.parse().unwrap();
            infrarust_config::secrets::redact(
                &mut document,
                infrarust_config::secrets::PROXY_SECRETS,
            );
            document.to_string()
        })
        .effective_with(|document| document.replace("./servers", OVERRIDDEN_SERVERS_DIR))
        .merging_writes(|submitted, stored| {
            let mut document: toml_edit::DocumentMut = submitted
                .parse()
                .map_err(|e: toml_edit::TomlError| ConfigWriteError::Parse(e.to_string()))?;
            let current: toml_edit::DocumentMut = stored.parse().unwrap();
            infrarust_config::secrets::reinject(
                &mut document,
                &current,
                infrarust_config::secrets::PROXY_SECRETS,
            );
            let text = document.to_string();
            toml::from_str::<infrarust_config::ProxyConfig>(&text)
                .map_err(|e| ConfigWriteError::Parse(e.to_string()))?;
            Ok(text)
        })
}

fn load_balancer() -> MockLoadBalancerService {
    let mut balancer = MockLoadBalancerService::new();
    for n in 0..2u8 {
        let id = format!("server_{n}");
        let address = ServerAddress {
            host: format!("10.0.0.{n}"),
            port: 25565,
        };
        balancer = balancer.with_server(id.as_str(), "least_conn", [address.clone()]);
        balancer
            .set_status(
                &ServerId::new(&id),
                BackendStatus {
                    address,
                    weight: 2,
                    effective_weight: 1,
                    state: BackendState::Healthy,
                    active_connections: 4,
                    healthy_since_secs: Some(30),
                    ejections: 1,
                    last_failure_secs_ago: Some(90),
                },
            )
            .unwrap();
    }
    balancer
}

fn plugin_registry() -> MockPluginRegistry {
    MockPluginRegistry::new().with_plugin(PluginInfo {
        id: "admin_api".to_owned(),
        name: "Admin API".to_owned(),
        version: "0.1.0".to_owned(),
        authors: vec!["Test".to_owned()],
        description: Some("Test plugin".to_owned()),
        state: "enabled".to_owned(),
        dependencies: vec![PluginDependencyInfo {
            id: "core".to_owned(),
            optional: false,
        }],
    })
}

pub struct TestApiBuilder {
    key: String,
    requests_per_minute: u64,
    config: Option<Arc<MockConfigService>>,
    ban_service: Option<Arc<MockBanService>>,
    players: Vec<Arc<MockPlayer>>,
    logs: Option<LogBroadcast>,
    files: Vec<(&'static str, &'static str)>,
}

impl TestApiBuilder {
    #[must_use]
    pub fn key(mut self, key: &str) -> Self {
        self.key = key.to_owned();
        self
    }

    #[must_use]
    pub const fn rate_limit(mut self, requests_per_minute: u64) -> Self {
        self.requests_per_minute = requests_per_minute;
        self
    }

    #[must_use]
    pub fn config(mut self, config: Arc<MockConfigService>) -> Self {
        self.config = Some(config);
        self
    }

    #[must_use]
    pub fn ban_service(mut self, ban_service: Arc<MockBanService>) -> Self {
        self.ban_service = Some(ban_service);
        self
    }

    #[must_use]
    pub fn players(mut self, players: impl IntoIterator<Item = Arc<MockPlayer>>) -> Self {
        self.players.extend(players);
        self
    }

    #[must_use]
    pub fn logs(mut self, logs: LogBroadcast) -> Self {
        self.logs = Some(logs);
        self
    }

    #[must_use]
    pub fn server_file(mut self, name: &'static str, contents: &'static str) -> Self {
        self.files.push((name, contents));
        self
    }

    pub fn build(self) -> TestApi {
        let dir = tempfile::tempdir().unwrap();
        let servers = dir.path().join("servers");
        for (name, contents) in &self.files {
            std::fs::create_dir_all(&servers).unwrap();
            std::fs::write(servers.join(name), contents).unwrap();
        }
        let config = self.config.unwrap_or_else(|| Arc::new(config_service()));
        let (event_tx, _) = broadcast::channel::<ApiEvent>(16);
        let player_registry = MockPlayerRegistry::new().fake_online_count(3);
        for player in self.players {
            player_registry.add(player);
        }
        let state = Arc::new(ApiState {
            player_registry: Arc::new(player_registry),
            ban_service: self
                .ban_service
                .unwrap_or_else(|| Arc::new(MockBanService::new())),
            server_manager: Arc::new(MockServerManager::new()),
            config_service: Arc::clone(&config) as _,
            load_balancer: Arc::new(load_balancer()),
            plugin_registry: Arc::new(plugin_registry()),
            config: ApiConfig {
                bind: "127.0.0.1:0".into(),
                api_key: self.key.clone(),
                cors_origins: vec![],
                rate_limit: RateLimitConfig::default(),
            },
            start_time: Instant::now(),
            proxy_version: "2.0.0-test".into(),
            rate_limiter: RateLimiter::new(self.requests_per_minute),
            event_tx,
            shutdown: CancellationToken::new(),
            proxy_shutdown: CancellationToken::new(),
            logs: self.logs,
            server_dir: Arc::new(ServerDir::open(dir.path()).unwrap()),
            provider_sender: Arc::new(tokio::sync::Mutex::new(None)),
            health_cache: Arc::new(HealthCache::new()),
            health_checker: Arc::new(HealthChecker::new()),
            recent_events: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            drain_store: Arc::new(DrainStore::open(dir.path())),
        });
        TestApi {
            state,
            config,
            key: self.key,
            dir,
        }
    }
}

pub struct TestApi {
    pub state: Arc<ApiState>,
    pub config: Arc<MockConfigService>,
    key: String,
    dir: tempfile::TempDir,
}

impl TestApi {
    #[must_use]
    pub fn builder() -> TestApiBuilder {
        TestApiBuilder {
            key: "test-key".to_owned(),
            requests_per_minute: 1000,
            config: None,
            ban_service: None,
            players: Vec::new(),
            logs: None,
            files: Vec::new(),
        }
    }

    #[must_use]
    pub fn new() -> Self {
        Self::builder().build()
    }

    #[must_use]
    pub fn with_key(key: &str) -> Self {
        Self::builder().key(key).build()
    }

    #[must_use]
    pub fn with_rate_limit(requests_per_minute: u64) -> Self {
        Self::builder().rate_limit(requests_per_minute).build()
    }

    #[must_use]
    pub fn with_config(config: Arc<MockConfigService>) -> Self {
        Self::builder().config(config).build()
    }

    #[must_use]
    pub fn managed_server() -> Self {
        Self::builder()
            .server_file("survival.toml", MANAGED_SERVER)
            .build()
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn router(&self) -> Router {
        build_router(Arc::clone(&self.state), true)
    }

    #[must_use]
    pub fn auth_header(&self) -> (HeaderName, HeaderValue) {
        (
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.key)).unwrap(),
        )
    }

    #[must_use]
    pub fn request(&self, method: http::Method, uri: &str) -> http::request::Builder {
        let (name, value) = self.auth_header();
        Request::builder()
            .method(method)
            .uri(uri)
            .header(name, value)
    }

    pub async fn send(&self, request: Request<Body>) -> Response<Body> {
        self.router().oneshot(request).await.unwrap()
    }

    pub async fn json(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = self.send(request).await;
        let status = response.status();
        (status, json_body(response).await)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let request = self
            .request(http::Method::GET, uri)
            .body(Body::empty())
            .unwrap();
        self.json(request).await
    }

    pub async fn delete(&self, uri: &str) -> (StatusCode, Value) {
        let request = self
            .request(http::Method::DELETE, uri)
            .body(Body::empty())
            .unwrap();
        self.json(request).await
    }

    pub async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.json(self.json_request(http::Method::POST, uri, &body))
            .await
    }

    pub async fn put(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.json(self.json_request(http::Method::PUT, uri, &body))
            .await
    }

    pub async fn text(
        &self,
        method: http::Method,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, Option<HeaderValue>, String) {
        let mut builder = self.request(method, uri);
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "text/plain");
        }
        let request = builder
            .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
            .unwrap();
        let response = self.send(request).await;
        let status = response.status();
        let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
        (status, content_type, text_body(response).await)
    }

    fn json_request(&self, method: http::Method, uri: &str, body: &Value) -> Request<Body> {
        self.request(method, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap()
    }
}

pub fn unauthenticated(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

pub async fn text_body(response: Response<Body>) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

pub async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_str(&text_body(response).await).unwrap()
}
