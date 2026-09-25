use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::{ProxyInitializeEvent, ProxyShutdownEvent};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_api::services::proxy_info::{
    KeepaliveInfo, ProxyInfo, RateLimitInfo, StatusCacheInfo, UnknownDomainBehavior,
};
use infrarust_api::services::server_manager::ServerManager;
use infrarust_config::ProxyConfig;
use infrarust_config::proxy::PluginConfig;
use infrarust_transport::Listener;

use crate::error::CoreError;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::plugin::manager::{PluginManager, PluginServices};
use crate::plugin::{
    PluginContextFactoryImpl, PluginLoader, PluginPermissions, PluginRegistryImpl,
};
use crate::server::{DEFAULT_DRAIN_TIMEOUT, ProxyServer};
use crate::services::ProxyServices;
use crate::services::config_service::ConfigServiceImpl;
use crate::services::scheduler::SchedulerImpl;
use crate::services::server_manager_bridge::{NoopServerManager, ServerManagerBridge};

pub struct ProxyRuntime;

impl ProxyRuntime {
    pub fn builder(config: ProxyConfig, config_path: PathBuf) -> ProxyRuntimeBuilder {
        ProxyRuntimeBuilder {
            config,
            config_path,
            shutdown: CancellationToken::new(),
            loaders: Vec::new(),
            trusted: Vec::new(),
            proxy_info: None,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
        }
    }
}

pub struct ProxyRuntimeBuilder {
    config: ProxyConfig,
    config_path: PathBuf,
    shutdown: CancellationToken,
    loaders: Vec<Box<dyn PluginLoader>>,
    trusted: Vec<String>,
    proxy_info: Option<ProxyInfo>,
    drain_timeout: Duration,
}

impl ProxyRuntimeBuilder {
    #[must_use]
    pub fn shutdown_token(mut self, token: CancellationToken) -> Self {
        self.shutdown = token;
        self
    }

    #[must_use]
    pub fn loader(mut self, loader: Box<dyn PluginLoader>) -> Self {
        self.loaders.push(loader);
        self
    }

    #[must_use]
    pub fn trusted_plugins(mut self, ids: impl IntoIterator<Item = String>) -> Self {
        self.trusted.extend(ids);
        self
    }

    #[must_use]
    pub fn proxy_info(mut self, info: ProxyInfo) -> Self {
        self.proxy_info = Some(info);
        self
    }

    #[must_use]
    pub fn drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    pub async fn start(self) -> Result<RunningProxy, CoreError> {
        let Self {
            config,
            config_path,
            shutdown,
            loaders,
            trusted,
            proxy_info,
            drain_timeout,
        } = self;

        let proxy_info = proxy_info
            .unwrap_or_else(|| proxy_info_from_config(&config, env!("CARGO_PKG_VERSION")));
        let plugins_dir = config.plugins_dir.clone();
        let plugin_cfgs = config.plugins.clone();

        let mut server = ProxyServer::new(config, config_path, shutdown.clone()).await?;

        let mut plugin_manager = PluginManager::new(loaders);
        plugin_manager.set_event_bus(Arc::clone(&server.services().event_bus));
        plugin_manager.set_disabled_plugins(
            plugin_cfgs
                .iter()
                .filter(|(_, c)| !c.enabled)
                .map(|(id, _)| id.clone())
                .collect(),
        );

        plugin_manager
            .discover_all(&plugins_dir)
            .await
            .map_err(|e| CoreError::Other(format!("failed to discover plugins: {e}")))?;

        let services = server.services();
        let transport_filter_registry = Arc::new(TransportFilterRegistryImpl::new());
        let plugin_registry = Arc::new(PluginRegistryImpl::new());
        let start_time = Instant::now();

        crate::commands::register_builtin_commands(
            &services.command_manager,
            services,
            Arc::clone(&plugin_registry) as Arc<dyn PluginRegistry>,
            start_time,
        );

        let plugin_services = plugin_services(
            services,
            &plugin_registry,
            &transport_filter_registry,
            shutdown.clone(),
            proxy_info,
            plugins_dir,
        );
        let context_factory = PluginContextFactoryImpl::new(
            plugin_services,
            plugin_permissions(plugin_cfgs, trusted),
        )
        .with_ban_providers(Arc::clone(&services.ban_manager))
        .with_permissions(Arc::clone(&services.permission_service))
        .with_limbo_handlers(Arc::clone(&services.limbo_handler_registry))
        .with_messaging(
            Arc::clone(&services.plugin_messaging),
            Arc::clone(&services.connection_registry),
        );

        let errors = plugin_manager.load_and_enable_all(&context_factory).await;
        if !errors.is_empty() {
            tracing::warn!(count = errors.len(), "Some plugins failed to enable");
        }
        services.ban_manager.report_missing_provider();
        services.permission_service.report_missing_provider();

        plugin_registry.update_from(&plugin_manager.list_plugins(), &|id| {
            plugin_manager.plugin_state(id).cloned()
        });

        activate_config_providers(&plugin_manager, services, server.background_token()).await;

        server.rebuild_transport_filter_chain(&transport_filter_registry);

        let plugin_manager = Arc::new(RwLock::new(plugin_manager));
        let server = Arc::new(server);

        let (listener, local_addr) = match bind_listener(&server).await {
            Ok(bound) => bound,
            Err(e) => {
                plugin_manager.write().await.shutdown().await;
                return Err(e);
            }
        };

        server.event_bus().fire(ProxyInitializeEvent).await;

        let serve_task = tokio::spawn(Arc::clone(&server).serve(listener));

        Ok(RunningProxy {
            server,
            plugin_manager,
            plugin_registry,
            context_factory,
            serve_task,
            local_addr,
            start_time,
            shutdown,
            drain_timeout,
        })
    }
}

fn plugin_services(
    services: &ProxyServices,
    plugin_registry: &Arc<PluginRegistryImpl>,
    transport_filter_registry: &Arc<TransportFilterRegistryImpl>,
    proxy_shutdown: CancellationToken,
    proxy_info: ProxyInfo,
    plugins_dir: PathBuf,
) -> PluginServices {
    let server_manager: Arc<dyn ServerManager> = match &services.server_manager {
        Some(sm) => Arc::new(ServerManagerBridge::new(Arc::clone(sm))),
        None => Arc::new(NoopServerManager),
    };

    PluginServices {
        event_bus: Arc::clone(&services.event_bus),
        player_registry: Arc::clone(&services.player_registry) as Arc<dyn PlayerRegistry>,
        server_manager,
        ban_service: Arc::clone(&services.ban_manager) as _,
        command_manager: Arc::clone(&services.command_manager),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(ConfigServiceImpl::new(
            Arc::clone(&services.domain_router),
            services.config_path.clone(),
            Arc::clone(&services.config),
        )),
        load_balancer_service: Arc::clone(&services.load_balancer_service) as _,
        plugin_registry: Arc::clone(plugin_registry) as Arc<dyn PluginRegistry>,
        codec_filter_registry: Arc::clone(&services.codec_filter_registry),
        transport_filter_registry: Arc::clone(transport_filter_registry),
        domain_router: Arc::clone(&services.domain_router),
        proxy_shutdown,
        proxy_info,
        plugins_dir,
    }
}

fn plugin_permissions(
    plugin_cfgs: HashMap<String, PluginConfig>,
    trusted: Vec<String>,
) -> HashMap<String, PluginPermissions> {
    let mut permissions: HashMap<String, PluginPermissions> = plugin_cfgs
        .into_iter()
        .map(|(id, c)| {
            (
                id,
                PluginPermissions {
                    permissions: c.permissions,
                    deny: c.deny,
                    trusted: false,
                },
            )
        })
        .collect();
    for id in trusted {
        permissions
            .entry(id)
            .and_modify(|p| p.trusted = true)
            .or_insert(PluginPermissions {
                permissions: Vec::new(),
                deny: Vec::new(),
                trusted: true,
            });
    }
    permissions
}

async fn activate_config_providers(
    plugin_manager: &PluginManager,
    services: &ProxyServices,
    shutdown: &CancellationToken,
) {
    let plugin_providers = plugin_manager.collect_config_providers();
    if plugin_providers.is_empty() {
        return;
    }
    tracing::info!(
        count = plugin_providers.len(),
        "activating plugin config providers"
    );
    let results = crate::provider::plugin_adapter::activate_plugin_providers(
        plugin_providers,
        services.provider_event_sender.clone(),
        &services.domain_router,
        shutdown.clone(),
    )
    .await;
    plugin_manager.store_provider_cleanup(results);
}

async fn bind_listener(server: &ProxyServer) -> Result<(Listener, SocketAddr), CoreError> {
    let listener = server.bind().await?;
    let local_addr = listener.local_addr()?;
    Ok((listener, local_addr))
}

pub fn proxy_info_from_config(config: &ProxyConfig, version: &str) -> ProxyInfo {
    ProxyInfo {
        version: version.to_string(),
        bind: config.bind,
        max_connections: config.max_connections,
        connect_timeout: config.connect_timeout,
        receive_proxy_protocol: config.receive_proxy_protocol,
        worker_threads: config.worker_threads,
        so_reuseport: config.so_reuseport,
        rate_limit: RateLimitInfo {
            max_connections: config.rate_limit.max_connections,
            window: config.rate_limit.window,
            status_max: config.rate_limit.status_max,
            status_window: config.rate_limit.status_window,
        },
        status_cache: StatusCacheInfo {
            ttl: config.status_cache.ttl,
            max_entries: config.status_cache.max_entries,
        },
        keepalive: KeepaliveInfo {
            time: config.keepalive.time,
            interval: config.keepalive.interval,
            retries: config.keepalive.retries,
        },
        telemetry_enabled: config.telemetry.as_ref().is_some_and(|t| t.enabled),
        docker_enabled: config.docker.is_some(),
        web_api_enabled: config.web.as_ref().is_some_and(|w| w.enable_api),
        web_ui_enabled: config.web.as_ref().is_some_and(|w| w.webui_enabled()),
        unknown_domain_behavior: match config.unknown_domain_behavior {
            infrarust_config::UnknownDomainBehavior::DefaultMotd => {
                UnknownDomainBehavior::DefaultMotd
            }
            infrarust_config::UnknownDomainBehavior::Drop => UnknownDomainBehavior::Drop,
        },
    }
}

pub struct RunningProxy {
    server: Arc<ProxyServer>,
    plugin_manager: Arc<RwLock<PluginManager>>,
    plugin_registry: Arc<PluginRegistryImpl>,
    context_factory: PluginContextFactoryImpl,
    serve_task: JoinHandle<Result<(), CoreError>>,
    local_addr: SocketAddr,
    start_time: Instant,
    shutdown: CancellationToken,
    drain_timeout: Duration,
}

impl RunningProxy {
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn server(&self) -> &Arc<ProxyServer> {
        &self.server
    }

    pub fn services(&self) -> &ProxyServices {
        self.server.services()
    }

    pub fn plugin_manager(&self) -> &Arc<RwLock<PluginManager>> {
        &self.plugin_manager
    }

    pub fn plugin_registry(&self) -> &Arc<PluginRegistryImpl> {
        &self.plugin_registry
    }

    pub fn service_registry(&self) -> &Arc<crate::plugin::service_registry::ServiceRegistryImpl> {
        self.context_factory.service_registry()
    }

    pub async fn disable_plugin(&self, id: &str) -> Result<(), infrarust_api::error::PluginError> {
        let mut manager = self.plugin_manager.write().await;
        manager.disable_plugin(id).await?;
        self.plugin_registry
            .update_from(&manager.list_plugins(), &|id| {
                manager.plugin_state(id).cloned()
            });
        Ok(())
    }

    pub const fn start_time(&self) -> Instant {
        self.start_time
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }

    pub async fn wait(mut self) -> Result<(), CoreError> {
        let result = match (&mut self.serve_task).await {
            Ok(result) => result,
            Err(e) => Err(CoreError::Other(format!("proxy server task failed: {e}"))),
        };

        self.server.close_sessions();
        self.server.drain_connections(self.drain_timeout).await;

        let bus = self.server.event_bus();
        bus.fire(ProxyShutdownEvent).await;
        bus.flush().await;

        self.plugin_manager.write().await.shutdown().await;
        self.server.stop_background_tasks();

        result
    }

    pub async fn shutdown(self) -> Result<(), CoreError> {
        self.shutdown.cancel();
        self.wait().await
    }
}

impl Drop for RunningProxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.server.close_sessions();
        self.server.stop_background_tasks();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Mutex;

    use infrarust_api::error::PluginError;
    use infrarust_api::event::bus::EventBusExt;
    use infrarust_api::event::{BoxFuture, EventPriority};
    use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};

    use crate::plugin::{PluginState, StaticPluginLoader};

    use super::*;

    type Log = Arc<Mutex<Vec<&'static str>>>;

    struct RecordingPlugin {
        id: &'static str,
        log: Log,
    }

    impl Plugin for RecordingPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new(self.id, self.id, "1.0.0")
        }

        fn on_enable<'a>(
            &'a self,
            ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            let log = Arc::clone(&self.log);
            Box::pin(async move {
                log.lock().unwrap().push("enable");
                let on_init = Arc::clone(&log);
                ctx.event_bus().subscribe(
                    EventPriority::NORMAL,
                    move |_: &mut ProxyInitializeEvent| on_init.lock().unwrap().push("initialize"),
                );
                let on_shutdown = Arc::clone(&log);
                ctx.event_bus().subscribe(
                    EventPriority::NORMAL,
                    move |_: &mut ProxyShutdownEvent| {
                        on_shutdown.lock().unwrap().push("shutdown");
                    },
                );
                Ok(())
            })
        }

        fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
            self.log.lock().unwrap().push("disable");
            Box::pin(async { Ok(()) })
        }
    }

    fn recording_loader(id: &'static str, log: &Log) -> StaticPluginLoader {
        let loader = StaticPluginLoader::new();
        let log = Arc::clone(log);
        loader.register(PluginMetadata::new(id, id, "1.0.0"), move || {
            Box::new(RecordingPlugin {
                id,
                log: Arc::clone(&log),
            })
        });
        loader
    }

    fn test_config(dir: &std::path::Path) -> ProxyConfig {
        let servers_dir = dir.join("servers");
        let plugins_dir = dir.join("plugins");
        std::fs::create_dir(&servers_dir).unwrap();
        std::fs::create_dir(&plugins_dir).unwrap();

        let mut config: ProxyConfig = toml::from_str("bind = \"127.0.0.1:0\"\n").unwrap();
        config.servers_dir = servers_dir;
        config.plugins_dir = plugins_dir;
        config.ban.file = dir.join("bans.json");
        config
    }

    fn assert_send<T: Send>(_: &T) {}

    #[tokio::test]
    async fn start_binds_fires_lifecycle_events_and_shuts_plugins_down() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let log: Log = Arc::new(Mutex::new(Vec::new()));

        let start = ProxyRuntime::builder(config, dir.path().join("infrarust.toml"))
            .loader(Box::new(recording_loader("recorder", &log)))
            .trusted_plugins(["recorder".to_string()])
            .drain_timeout(Duration::from_secs(1))
            .start();
        assert_send(&start);
        let running = start.await.unwrap();

        assert_ne!(running.local_addr().port(), 0);
        assert_eq!(*log.lock().unwrap(), vec!["enable", "initialize"]);
        assert!(
            running
                .plugin_manager()
                .read()
                .await
                .is_plugin_loaded("recorder")
        );
        assert!(running.plugin_registry().plugin_info("recorder").is_some());

        let client = tokio::net::TcpStream::connect(running.local_addr()).await;
        assert!(client.is_ok(), "the proxy should accept TCP connections");
        drop(client);

        let shutdown = running.shutdown();
        assert_send(&shutdown);
        shutdown.await.unwrap();

        assert_eq!(
            *log.lock().unwrap(),
            vec!["enable", "initialize", "shutdown", "disable"]
        );
    }

    struct LateGate;

    impl infrarust_api::limbo::LimboHandler for LateGate {
        fn name(&self) -> &str {
            "late_gate"
        }

        fn on_player_enter<'a>(
            &'a self,
            _session: &'a dyn infrarust_api::limbo::LimboSession,
        ) -> BoxFuture<'a, infrarust_api::limbo::HandlerResult> {
            Box::pin(async { infrarust_api::limbo::HandlerResult::Hold })
        }
    }

    #[tokio::test]
    async fn a_limbo_handler_registered_after_startup_reaches_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let running = ProxyRuntime::builder(test_config(dir.path()), dir.path().join("i.toml"))
            .loader(Box::new(recording_loader("late", &log)))
            .trusted_plugins(["late".to_string()])
            .start()
            .await
            .unwrap();

        let ctx = running
            .plugin_manager()
            .read()
            .await
            .plugin_context("late")
            .unwrap();
        ctx.register_limbo_handler(Box::new(LateGate)).unwrap();

        assert!(
            running
                .services()
                .limbo_handler_registry
                .get("late_gate")
                .is_some()
        );
        running.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn plugin_disabled_in_config_is_not_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        config.plugins.insert(
            "sleeper".to_string(),
            PluginConfig {
                path: None,
                permissions: Vec::new(),
                deny: Vec::new(),
                wasm: None,
                strict_capabilities: false,
                enabled: false,
            },
        );
        let log: Log = Arc::new(Mutex::new(Vec::new()));

        let running = ProxyRuntime::builder(config, dir.path().join("infrarust.toml"))
            .loader(Box::new(recording_loader("sleeper", &log)))
            .start()
            .await
            .unwrap();

        {
            let manager = running.plugin_manager().read().await;
            assert!(!manager.is_plugin_loaded("sleeper"));
            assert!(matches!(
                manager.plugin_state("sleeper"),
                Some(PluginState::Disabled)
            ));
        }
        assert!(log.lock().unwrap().is_empty());

        running.shutdown().await.unwrap();
        assert!(log.lock().unwrap().is_empty());
    }
}
