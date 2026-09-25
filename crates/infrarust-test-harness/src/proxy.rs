use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use infrarust_api::error::PluginError;
use infrarust_api::event::BoxFuture;
use infrarust_api::player::Player;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_config::{ProxyConfig, ProxyMode, ServerConfig};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::{PluginLoader, PluginState, StaticPluginLoader};
use infrarust_core::runtime::{ProxyRuntime, RunningProxy};
use infrarust_core::server::ProxyServer;
use infrarust_core::services::ProxyServices;
use infrarust_protocol::version::ProtocolVersion;
use tempfile::TempDir;
use tokio::net::TcpSocket;
use tokio_util::sync::CancellationToken;
use toml::{Table, Value};

use crate::client::FakeClient;
use crate::error::{HarnessError, HarnessResult};
use crate::legacy::LegacyClient;
use crate::session::FakeSessionServer;

const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CONFIG_FILE: &str = "infrarust.toml";

type TablePatch = Box<dyn FnOnce(&mut Table) + Send>;

pub struct ServerSpec {
    id: String,
    mode: ProxyMode,
    backends: Vec<SocketAddr>,
    unreachable: bool,
    domains: Option<Vec<String>>,
    network: Option<String>,
    limbo_handlers: Vec<String>,
    patches: Vec<TablePatch>,
}

impl std::fmt::Debug for ServerSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSpec")
            .field("id", &self.id)
            .field("mode", &self.mode)
            .field("backends", &self.backends)
            .field("unreachable", &self.unreachable)
            .field("domains", &self.domains)
            .field("network", &self.network)
            .field("limbo_handlers", &self.limbo_handlers)
            .finish_non_exhaustive()
    }
}

impl ServerSpec {
    pub fn new(id: impl Into<String>, mode: ProxyMode) -> Self {
        Self {
            id: id.into(),
            mode,
            backends: Vec::new(),
            unreachable: false,
            domains: None,
            network: None,
            limbo_handlers: Vec::new(),
            patches: Vec::new(),
        }
    }

    pub fn offline(id: impl Into<String>) -> Self {
        Self::new(id, ProxyMode::Offline)
    }

    pub fn client_only(id: impl Into<String>) -> Self {
        Self::new(id, ProxyMode::ClientOnly)
    }

    pub fn passthrough(id: impl Into<String>) -> Self {
        Self::new(id, ProxyMode::Passthrough)
    }

    pub fn zero_copy(id: impl Into<String>) -> Self {
        Self::new(id, ProxyMode::ZeroCopy)
    }

    pub fn server_only(id: impl Into<String>) -> Self {
        Self::new(id, ProxyMode::ServerOnly)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn backend(mut self, addr: SocketAddr) -> Self {
        self.backends.push(addr);
        self
    }

    #[must_use]
    pub fn backends(mut self, addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        self.backends.extend(addrs);
        self
    }

    #[must_use]
    pub const fn unreachable(mut self) -> Self {
        self.unreachable = true;
        self
    }

    #[must_use]
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domains
            .get_or_insert_with(Vec::new)
            .push(domain.into());
        self
    }

    #[must_use]
    pub fn network(mut self, network: impl Into<String>) -> Self {
        self.network = Some(network.into());
        self
    }

    #[must_use]
    pub fn limbo_handlers<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.limbo_handlers
            .extend(names.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn patch(mut self, patch: impl FnOnce(&mut Table) + Send + 'static) -> Self {
        self.patches.push(Box::new(patch));
        self
    }

    fn domain_list(&self) -> Vec<String> {
        self.domains
            .clone()
            .unwrap_or_else(|| vec![format!("{}.test", self.id)])
    }

    fn into_table(self, reserved: &mut Vec<TcpSocket>) -> HarnessResult<Table> {
        let mut addresses: Vec<Value> = self
            .backends
            .iter()
            .map(|addr| Value::String(addr.to_string()))
            .collect();
        if self.unreachable {
            let (addr, socket) = closed_address()?;
            reserved.push(socket);
            addresses.push(Value::String(addr.to_string()));
        }
        if addresses.is_empty() {
            return Err(HarnessError::setup(format!(
                "server {} has no backend: call .backend(addr) or .unreachable()",
                self.id
            )));
        }

        let mut table = Table::new();
        table.insert("domains".into(), string_array(self.domain_list()));
        table.insert("addresses".into(), Value::Array(addresses));
        table.insert(
            "proxy_mode".into(),
            Value::try_from(self.mode).map_err(HarnessError::setup)?,
        );
        if let Some(network) = self.network {
            table.insert("network".into(), Value::String(network));
        }
        if !self.limbo_handlers.is_empty() {
            table.insert("limbo_handlers".into(), string_array(self.limbo_handlers));
        }
        for patch in self.patches {
            patch(&mut table);
        }
        Ok(table)
    }
}

pub struct TestProxyBuilder {
    servers: Vec<ServerSpec>,
    plugins: Vec<Box<dyn Plugin>>,
    loaders: Vec<Box<dyn PluginLoader>>,
    session_url: Option<String>,
    patches: Vec<TablePatch>,
    drain_timeout: Duration,
}

impl std::fmt::Debug for TestProxyBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestProxyBuilder")
            .field("servers", &self.servers)
            .field("plugins", &self.plugins.len())
            .field("loaders", &self.loaders.len())
            .field("session_url", &self.session_url)
            .finish_non_exhaustive()
    }
}

impl TestProxyBuilder {
    #[must_use]
    pub fn server(mut self, spec: ServerSpec) -> Self {
        self.servers.push(spec);
        self
    }

    #[must_use]
    pub fn plugin(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    #[must_use]
    pub fn loader(mut self, loader: Box<dyn PluginLoader>) -> Self {
        self.loaders.push(loader);
        self
    }

    #[must_use]
    pub fn session_server(mut self, server: &FakeSessionServer) -> Self {
        self.session_url = Some(server.url());
        self
    }

    #[must_use]
    pub fn patch_config(mut self, patch: impl FnOnce(&mut Table) + Send + 'static) -> Self {
        self.patches.push(Box::new(patch));
        self
    }

    #[must_use]
    pub const fn drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    pub async fn start(self) -> HarnessResult<TestProxy> {
        crate::init_tracing();
        let Self {
            servers,
            plugins,
            loaders,
            session_url,
            patches,
            drain_timeout,
        } = self;

        let dir = tempfile::tempdir()?;
        let servers_dir = dir.path().join("servers");
        let plugins_dir = dir.path().join("plugins");
        std::fs::create_dir(&servers_dir)?;
        std::fs::create_dir(&plugins_dir)?;

        let mut reserved = Vec::new();
        let routes = write_servers(servers, &servers_dir, &mut reserved)?;

        let mut table = Table::new();
        table.insert("bind".into(), Value::String("127.0.0.1:0".into()));
        table.insert("servers_dir".into(), path_value(&servers_dir)?);
        table.insert("plugins_dir".into(), path_value(&plugins_dir)?);
        table.insert(
            "rate_limit".into(),
            Value::Table(Table::from_iter([(
                "enabled".into(),
                Value::Boolean(false),
            )])),
        );
        table.insert(
            "ban".into(),
            Value::Table(Table::from_iter([(
                "file".into(),
                path_value(&dir.path().join("bans.json"))?,
            )])),
        );
        if let Some(url) = session_url {
            table.insert(
                "auth".into(),
                Value::Table(Table::from_iter([(
                    "session_url".into(),
                    Value::String(url),
                )])),
            );
        }
        for patch in patches {
            patch(&mut table);
        }

        let config_path = dir.path().join(CONFIG_FILE);
        let text = toml::to_string(&table).map_err(HarnessError::setup)?;
        std::fs::write(&config_path, &text)?;
        let config: ProxyConfig = toml::from_str(&text)
            .map_err(|e| HarnessError::setup(format!("{CONFIG_FILE}: {e}")))?;
        infrarust_config::validate_proxy_config(&config)
            .map_err(|e| HarnessError::setup(format!("{CONFIG_FILE}: {e}")))?;

        let shutdown = CancellationToken::new();
        let mut builder = ProxyRuntime::builder(config, config_path)
            .shutdown_token(shutdown.clone())
            .drain_timeout(drain_timeout);
        let plugin_ids = if plugins.is_empty() {
            Vec::new()
        } else {
            let (loader, ids) = static_loader(plugins)?;
            builder = builder
                .loader(Box::new(loader))
                .trusted_plugins(ids.clone());
            ids
        };
        for loader in loaders {
            builder = builder.loader(loader);
        }

        let running = match builder.start().await {
            Ok(running) => running,
            Err(e) => {
                shutdown.cancel();
                return Err(e.into());
            }
        };

        let failures: Vec<String> = {
            let manager = running.plugin_manager().read().await;
            plugin_ids
                .iter()
                .filter_map(|id| match manager.plugin_state(id) {
                    Some(PluginState::Enabled) => None,
                    other => Some(format!("{id} ({other:?})")),
                })
                .collect()
        };
        if !failures.is_empty() {
            let _ = running.shutdown().await;
            return Err(HarnessError::setup(format!(
                "plugins failed to enable: {}",
                failures.join(", ")
            )));
        }

        Ok(TestProxy {
            addr: running.local_addr(),
            server: Arc::clone(running.server()),
            running: Some(running),
            shutdown,
            routes,
            dir,
            _reserved: reserved,
        })
    }
}

pub struct TestProxy {
    running: Option<RunningProxy>,
    server: Arc<ProxyServer>,
    shutdown: CancellationToken,
    addr: SocketAddr,
    routes: Vec<(String, String)>,
    dir: TempDir,
    _reserved: Vec<TcpSocket>,
}

impl std::fmt::Debug for TestProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestProxy")
            .field("addr", &self.addr)
            .field("routes", &self.routes)
            .field("dir", &self.dir.path())
            .finish_non_exhaustive()
    }
}

impl TestProxy {
    pub fn builder() -> TestProxyBuilder {
        TestProxyBuilder {
            servers: Vec::new(),
            plugins: Vec::new(),
            loaders: Vec::new(),
            session_url: None,
            patches: Vec::new(),
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
        }
    }

    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn services(&self) -> &ProxyServices {
        self.server.services()
    }

    pub fn bus(&self) -> &Arc<EventBusImpl> {
        &self.services().event_bus
    }

    pub const fn server(&self) -> &Arc<ProxyServer> {
        &self.server
    }

    pub const fn running(&self) -> Option<&RunningProxy> {
        self.running.as_ref()
    }

    pub const fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }

    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn config_path(&self) -> std::path::PathBuf {
        self.dir.path().join(CONFIG_FILE)
    }

    pub fn domain(&self, server_id: &str) -> Option<&str> {
        self.routes
            .iter()
            .find(|(id, _)| id == server_id)
            .map(|(_, domain)| domain.as_str())
    }

    pub fn client(&self, version: ProtocolVersion) -> FakeClient {
        let client = FakeClient::new(self.addr, version);
        match self.routes.first() {
            Some((_, domain)) => client.domain(domain.clone()),
            None => client,
        }
    }

    pub fn client_for(
        &self,
        server_id: &str,
        version: ProtocolVersion,
    ) -> HarnessResult<FakeClient> {
        let domain = self
            .domain(server_id)
            .ok_or_else(|| HarnessError::setup(format!("no server {server_id} in this proxy")))?;
        Ok(FakeClient::new(self.addr, version).domain(domain))
    }

    pub fn legacy_client_for(&self, server_id: &str) -> HarnessResult<LegacyClient> {
        let domain = self
            .domain(server_id)
            .ok_or_else(|| HarnessError::setup(format!("no server {server_id} in this proxy")))?;
        Ok(LegacyClient::new(self.addr).hostname(domain))
    }

    pub fn connection_count(&self) -> usize {
        self.services().connection_registry.count()
    }

    pub async fn wait_for_connection_count(
        &self,
        expected: usize,
        timeout: Duration,
    ) -> HarnessResult<()> {
        poll_until(
            &format!("the proxy to hold {expected} connection(s)"),
            timeout,
            || (self.connection_count() == expected).then_some(()),
        )
        .await
    }

    pub async fn wait_for_player(
        &self,
        username: &str,
        timeout: Duration,
    ) -> HarnessResult<Arc<dyn Player>> {
        let registry = &self.services().player_registry;
        poll_until(
            &format!("player {username} to be registered"),
            timeout,
            || registry.get_player(username),
        )
        .await
    }

    pub async fn shutdown(mut self) -> HarnessResult<()> {
        match self.running.take() {
            Some(running) => Ok(running.shutdown().await?),
            None => Ok(()),
        }
    }
}

impl Drop for TestProxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn poll_until<T>(
    what: &str,
    timeout: Duration,
    mut probe: impl FnMut() -> Option<T>,
) -> HarnessResult<T> {
    let mut ticks = tokio::time::interval(POLL_INTERVAL);
    tokio::time::timeout(timeout, async {
        loop {
            ticks.tick().await;
            if let Some(value) = probe() {
                return value;
            }
        }
    })
    .await
    .map_err(|_| HarnessError::timeout(what, timeout))
}

fn write_servers(
    servers: Vec<ServerSpec>,
    dir: &Path,
    reserved: &mut Vec<TcpSocket>,
) -> HarnessResult<Vec<(String, String)>> {
    let mut routes: Vec<(String, String)> = Vec::new();
    for spec in servers {
        let id = spec.id.clone();
        if routes.iter().any(|(existing, _)| *existing == id) {
            return Err(HarnessError::setup(format!("duplicate server id {id}")));
        }
        let table = spec.into_table(reserved)?;
        let text = toml::to_string(&table).map_err(HarnessError::setup)?;
        let mut parsed: ServerConfig =
            toml::from_str(&text).map_err(|e| HarnessError::setup(format!("server {id}: {e}")))?;
        parsed.id.get_or_insert_with(|| id.clone());
        infrarust_config::validate_server_config(&parsed)
            .map_err(|e| HarnessError::setup(format!("server {id}: {e}")))?;
        std::fs::write(dir.join(format!("{id}.toml")), text)?;
        let domain = parsed.domains.first().cloned().unwrap_or_default();
        routes.push((id, domain));
    }
    Ok(routes)
}

fn static_loader(
    plugins: Vec<Box<dyn Plugin>>,
) -> HarnessResult<(StaticPluginLoader, Vec<String>)> {
    let loader = StaticPluginLoader::new();
    let mut ids: Vec<String> = Vec::new();
    for plugin in plugins {
        let metadata = plugin.metadata();
        if ids.contains(&metadata.id) {
            return Err(HarnessError::setup(format!(
                "duplicate plugin id {}",
                metadata.id
            )));
        }
        ids.push(metadata.id.clone());
        let slot = Mutex::new(Some(plugin));
        let spent = metadata.clone();
        loader.register(metadata, move || {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| Box::new(SpentPlugin(spent.clone())))
        });
    }
    Ok((loader, ids))
}

struct SpentPlugin(PluginMetadata);

impl Plugin for SpentPlugin {
    fn metadata(&self) -> PluginMetadata {
        self.0.clone()
    }

    fn on_enable<'a>(
        &'a self,
        _ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        let id = self.0.id.clone();
        Box::pin(async move {
            Err(PluginError::InitFailed(format!(
                "the harness instance of plugin {id} was already handed out"
            )))
        })
    }
}

fn closed_address() -> HarnessResult<(SocketAddr, TcpSocket)> {
    let socket = TcpSocket::new_v4()?;
    socket.set_reuseaddr(false)?;
    socket.bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    Ok((socket.local_addr()?, socket))
}

fn path_value(path: &Path) -> HarnessResult<Value> {
    path.to_str()
        .map(|s| Value::String(s.to_string()))
        .ok_or_else(|| HarnessError::setup(format!("non UTF-8 path {}", path.display())))
}

fn string_array(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}
