use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::{ProxyInitializeEvent, ProxyShutdownEvent};
use infrarust_api::player::Player;
use infrarust_api::services::ProxyUpdates;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::plugin_registry::PluginRegistry;
use infrarust_api::services::proxy_info::{
    KeepaliveInfo, ProxyInfo, RateLimitInfo, StatusCacheInfo, UnknownDomainBehavior,
};
use infrarust_api::services::server_manager::ServerManager;
use infrarust_config::ProxyConfig;
use infrarust_config::proxy::PluginConfig;
use infrarust_server_manager::ServerProvider;
use infrarust_transport::Listener;

use crate::binary::RunningBinary;
use crate::console::ConsoleServices;
use crate::error::CoreError;
use crate::hot_restart::restore::{self, Inherited, Restored};
use crate::hot_restart::{
    HandoverPackage, Inheritance, ListenerHandover, Preflight, PreflightError, RestartHub,
    RestartPhase, RestartPlan, UnpackFailure,
};
use crate::pipeline::context::connection_id_watermark;
use crate::player::ShutdownKind;
use crate::plugin::manager::{PluginManager, PluginServices, ShutdownLimits};
use crate::plugin::{
    HostWiring, PluginContextFactoryImpl, PluginLoader, PluginManagerError, PluginManagerHandle,
    PluginPermissions, PluginRegistryImpl,
};
use crate::provider::plugin_adapter::PluginProviderActivator;
use crate::server::ProxyServices;
use crate::server::{DEFAULT_DRAIN_TIMEOUT, ProxyServer, ServeExit};
use crate::services::config_service::ConfigServiceImpl;
use crate::services::scheduler::SchedulerImpl;
use crate::services::server_manager_bridge::{NoopServerManager, ServerManagerBridge};
use crate::update::UpdateService;

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
            plugin_shutdown: ShutdownLimits::default(),
            server_providers: Vec::new(),
            inherited_listener: None,
            binary: None,
            hot_restart: None,
            restored: None,
        }
    }
}

struct ProviderOverride {
    server_id: String,
    provider: Arc<dyn ServerProvider>,
    start_timeout: Duration,
    poll_interval: Duration,
}

pub struct ProxyRuntimeBuilder {
    config: ProxyConfig,
    config_path: PathBuf,
    shutdown: CancellationToken,
    loaders: Vec<Box<dyn PluginLoader>>,
    trusted: Vec<String>,
    proxy_info: Option<ProxyInfo>,
    drain_timeout: Duration,
    plugin_shutdown: ShutdownLimits,
    server_providers: Vec<ProviderOverride>,
    inherited_listener: Option<std::net::TcpListener>,
    binary: Option<Arc<RunningBinary>>,
    hot_restart: Option<Arc<dyn Preflight>>,
    restored: Option<Box<dyn Inheritance>>,
}

impl ProxyRuntimeBuilder {
    #[must_use]
    pub fn binary(mut self, binary: Option<Arc<RunningBinary>>) -> Self {
        self.binary = binary;
        self
    }

    #[must_use]
    pub fn hot_restart(mut self, preflight: Arc<dyn Preflight>) -> Self {
        self.hot_restart = Some(preflight);
        self
    }

    #[must_use]
    pub fn restored(mut self, inheritance: Box<dyn Inheritance>) -> Self {
        self.restored = Some(inheritance);
        self
    }

    #[must_use]
    pub fn inherited_listener(mut self, listener: Option<std::net::TcpListener>) -> Self {
        self.inherited_listener = listener;
        self
    }

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

    #[must_use]
    pub fn plugin_disable_timeout(mut self, timeout: Duration) -> Self {
        self.plugin_shutdown.on_disable = timeout;
        self
    }

    #[must_use]
    pub fn plugin_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.plugin_shutdown.total = timeout;
        self
    }

    #[must_use]
    pub fn server_provider(
        mut self,
        server_id: impl Into<String>,
        provider: Arc<dyn ServerProvider>,
        start_timeout: Duration,
        poll_interval: Duration,
    ) -> Self {
        self.server_providers.push(ProviderOverride {
            server_id: server_id.into(),
            provider,
            start_timeout,
            poll_interval,
        });
        self
    }

    pub async fn start(mut self) -> Result<RunningProxy, CoreError> {
        let Some(mut inheritance) = self.restored.take() else {
            return self.boot(None).await;
        };
        let package = match inheritance.take_package() {
            Ok(package) => package,
            Err(error) => return Err(Succession { inheritance }.abort(error.into(), ())),
        };
        let succession = Succession { inheritance };
        match Inherited::unpack(package) {
            Ok(inherited) => {
                tracing::info!(
                    from = inherited.from_version(),
                    sessions = inherited.session_count(),
                    "taking over from the previous image"
                );
                self.boot(Some(Takeover {
                    succession,
                    inherited,
                }))
                .await
            }
            Err(UnpackFailure { error, package }) => Err(succession.abort(error.into(), package)),
        }
    }

    async fn boot(self, takeover: Option<Takeover>) -> Result<RunningProxy, CoreError> {
        let Self {
            config,
            config_path,
            shutdown,
            loaders,
            trusted,
            proxy_info,
            drain_timeout,
            plugin_shutdown,
            server_providers,
            mut inherited_listener,
            binary,
            hot_restart,
            restored: _,
        } = self;

        let proxy_info = proxy_info
            .unwrap_or_else(|| proxy_info_from_config(&config, env!("CARGO_PKG_VERSION")));
        let plugins_dir = config.plugins_dir.clone();
        let plugin_cfgs = config.plugins.clone();
        let hub = restart_hub(binary.as_ref(), hot_restart);
        let restart = hub.as_ref().map(RestartHub::handle);
        let updates = binary.as_ref().map(|binary| {
            UpdateService::new(config.update.clone(), Arc::clone(binary), restart.clone())
        });

        let server = match ProxyServer::new(config, config_path, shutdown.clone()).await {
            Ok(server) => server,
            Err(error) => return Err(Takeover::abandon(takeover, error)),
        };
        if let Err(error) = register_server_providers(&server, server_providers) {
            return Err(Takeover::abandon(takeover, error));
        }
        let (early_listener, restored) = match takeover {
            Some(takeover) => {
                let (listener, restored) = takeover
                    .complete(&server, inherited_listener.take())
                    .await?;
                (Some(listener), Some(restored))
            }
            None => (None, None),
        };

        let plugin_registry = Arc::new(PluginRegistryImpl::new());
        let mut plugin_manager = PluginManager::new(
            loaders,
            Arc::clone(&server.services().event_bus),
            Arc::clone(&plugin_registry),
        );
        plugin_manager.set_shutdown_limits(plugin_shutdown);
        plugin_manager.set_disabled_plugins(
            plugin_cfgs
                .iter()
                .filter(|(_, c)| !c.enabled)
                .map(|(id, _)| id.clone())
                .collect(),
        );

        if let Err(error) = plugin_manager.discover_all(&plugins_dir).await {
            if restored.is_none() {
                return Err(error.into());
            }
            tracing::error!(
                %error,
                "plugin discovery failed; the restored proxy keeps its sessions and runs without the plugins it could not load"
            );
        }

        let services = server.services();
        let start_time = Instant::now();

        crate::commands::register_builtin_commands(
            &services.command_manager,
            services,
            Arc::clone(&plugin_registry) as Arc<dyn PluginRegistry>,
            start_time,
            updates
                .clone()
                .map(|updates| updates as Arc<dyn ProxyUpdates>),
        );

        let plugin_services = plugin_services(
            services,
            &plugin_registry,
            shutdown.clone(),
            server.background_token().clone(),
            proxy_info,
            plugins_dir,
        );
        let provider_activator = Arc::clone(&plugin_services.provider_activator);
        let context_factory = Arc::new(PluginContextFactoryImpl::new(
            plugin_services,
            plugin_permissions(plugin_cfgs, trusted),
            HostWiring {
                ban_manager: Arc::clone(&services.ban_manager),
                permissions: Arc::clone(&services.permission_service),
                limbo_handlers: Arc::clone(&services.limbo_handler_registry),
                channels: Arc::clone(&services.plugin_messaging),
                players: Arc::clone(&services.connection_registry),
                updates: updates
                    .clone()
                    .map(|updates| updates as Arc<dyn ProxyUpdates>),
                restart,
            },
        ));

        let errors = plugin_manager
            .load_and_enable_all(Arc::clone(&context_factory))
            .await;
        if !errors.is_empty() {
            tracing::warn!(count = errors.len(), "Some plugins failed to enable");
        }
        services.ban_manager.report_missing_provider();
        services.permission_service.report_missing_provider();

        provider_activator.start();
        plugin_manager.activate_config_providers().await;
        if let Some(restored) = &restored {
            set_up_restored_permissions(services, restored).await;
        }

        let plugins = PluginManagerHandle::new(plugin_manager);
        let server = Arc::new(server);

        let (listener, local_addr) = match early_listener {
            Some(listener) => {
                let local_addr = listener.local_addr();
                (listener, local_addr)
            }
            None => match bind_listener(&server, inherited_listener).await {
                Ok(bound) => bound,
                Err(e) => {
                    plugins.shutdown().await;
                    return Err(e);
                }
            },
        };

        server.event_bus().fire(ProxyInitializeEvent::new()).await;

        let serve_task = tokio::spawn(Arc::clone(&server).serve(listener));
        if let Some(updates) = &updates {
            updates.spawn_background(server.background_token().clone());
        }

        Ok(RunningProxy {
            server,
            plugins,
            plugin_registry,
            context_factory,
            serve_task,
            local_addr,
            start_time,
            shutdown,
            drain_timeout,
            plugin_shutdown_timeout: plugin_shutdown.total,
            binary,
            updates,
            hub,
        })
    }
}

fn restart_hub(
    binary: Option<&Arc<RunningBinary>>,
    preflight: Option<Arc<dyn Preflight>>,
) -> Option<Arc<RestartHub>> {
    let preflight = preflight?;
    let Some(binary) = binary else {
        tracing::warn!(
            "hot restart needs the path of the running executable, which is unknown; it stays unavailable"
        );
        return None;
    };
    let plan = RestartPlan {
        binary: Arc::clone(binary),
        target: binary.exe.path().to_path_buf(),
    };
    Some(RestartHub::new(plan, preflight))
}

async fn set_up_restored_permissions(services: &ProxyServices, restored: &Restored) {
    for player in restored
        .players
        .iter()
        .filter(|player| player.is_connected())
    {
        player.setup_permissions(&services.event_bus).await;
        player.refresh_permissions().await;
    }
}

struct Succession {
    inheritance: Box<dyn Inheritance>,
}

impl Succession {
    fn abort<T>(self, error: CoreError, held: T) -> CoreError {
        let reason = error.to_string();
        tracing::error!(%reason, "this image cannot take over, handing the sessions back to the previous one");
        let returned = self.inheritance.roll_back(&reason);
        drop(held);
        CoreError::Handover(returned)
    }

    fn commit(self) {
        match self.inheritance.commit() {
            Ok(()) => tracing::info!("handover committed, this image owns the inherited sessions"),
            Err(error) => tracing::error!(
                %error,
                "the handover commit failed; resuming the inherited sessions anyway"
            ),
        }
    }
}

struct Takeover {
    succession: Succession,
    inherited: Inherited,
}

impl Takeover {
    fn abandon(takeover: Option<Self>, error: CoreError) -> CoreError {
        match takeover {
            Some(Self {
                succession,
                inherited,
            }) => succession.abort(error, inherited),
            None => error,
        }
    }

    async fn complete(
        self,
        server: &ProxyServer,
        activated: Option<std::net::TcpListener>,
    ) -> Result<(Listener, Restored), CoreError> {
        let Self {
            succession,
            mut inherited,
        } = self;
        let bind = server.services().config.bind;
        let adopted = inherited.take_listener(bind);
        if adopted.is_none()
            && let Some(previous) = inherited.previous_bind()
        {
            tracing::info!(
                %previous,
                %bind,
                "the configured bind changed; binding the new address and closing the inherited listener after the handover"
            );
        }
        let listener = match server.bind_or_adopt(adopted.or(activated)).await {
            Ok(listener) => listener,
            Err(error) => return Err(succession.abort(error, inherited)),
        };
        let prepared = match restore::prepare(server, inherited) {
            Ok(prepared) => prepared,
            Err(failure) => {
                let error = CoreError::Startup(failure.to_string());
                return Err(succession.abort(error, (failure, listener)));
            }
        };
        succession.commit();
        let restored = restore::launch(server, &listener, prepared);
        Ok((listener, restored))
    }
}

fn register_server_providers(
    server: &ProxyServer,
    providers: Vec<ProviderOverride>,
) -> Result<(), CoreError> {
    if providers.is_empty() {
        return Ok(());
    }
    let Some(manager) = server.services().server_manager.as_ref() else {
        return Err(CoreError::Startup(
            "a server provider was given but no server has a [server_manager] section".to_string(),
        ));
    };
    for provider in providers {
        if manager.get_state(&provider.server_id).is_none() {
            return Err(CoreError::Startup(format!(
                "a server provider was given for {}, which has no [server_manager] section",
                provider.server_id
            )));
        }
        manager.register_server(
            provider.server_id,
            provider.provider,
            None,
            provider.start_timeout,
            provider.poll_interval,
        );
    }
    Ok(())
}

fn plugin_services(
    services: &ProxyServices,
    plugin_registry: &Arc<PluginRegistryImpl>,
    proxy_shutdown: CancellationToken,
    background: CancellationToken,
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
        transport_filter_registry: Arc::clone(&services.transport_filter_registry),
        connection_claims: Arc::clone(&services.connection_claims),
        provider_activator: Arc::new(PluginProviderActivator::new(
            services.plugin_routes.clone(),
            background,
        )),
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

async fn bind_listener(
    server: &ProxyServer,
    inherited: Option<std::net::TcpListener>,
) -> Result<(Listener, SocketAddr), CoreError> {
    let listener = server.bind_or_adopt(inherited).await?;
    let local_addr = listener.local_addr();
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

const RESTART_GRACE: Duration = Duration::from_secs(3);
const RESTART_PIPELINE_DRAIN: Duration = Duration::from_secs(5);
const FREEZE_MARGIN: Duration = Duration::from_millis(400);
const FROZEN_TASKS: Duration = Duration::from_secs(1);
const WITHOUT_RESTART_LOOP: &str =
    "this proxy was started without its restart loop; restart it by hand";

#[derive(Debug)]
pub enum ProxyExit {
    Stopped,
    Restart(Box<RestartHandover>),
}

#[derive(Debug)]
pub struct RestartHandover {
    pub package: HandoverPackage,
    pub plan: RestartPlan,
}

enum Next {
    Served(Result<(), CoreError>),
    Requested,
}

enum Attempt {
    Aborted,
    Stop(Result<(), CoreError>),
    Handed(Box<RestartHandover>),
    Failed(CoreError),
}

pub struct RunningProxy {
    server: Arc<ProxyServer>,
    plugins: PluginManagerHandle,
    plugin_registry: Arc<PluginRegistryImpl>,
    context_factory: Arc<PluginContextFactoryImpl>,
    serve_task: JoinHandle<Result<ServeExit, CoreError>>,
    local_addr: SocketAddr,
    start_time: Instant,
    shutdown: CancellationToken,
    drain_timeout: Duration,
    plugin_shutdown_timeout: Duration,
    binary: Option<Arc<RunningBinary>>,
    updates: Option<Arc<UpdateService>>,
    hub: Option<Arc<RestartHub>>,
}

impl RunningProxy {
    pub const fn binary(&self) -> Option<&Arc<RunningBinary>> {
        self.binary.as_ref()
    }

    pub const fn updates(&self) -> Option<&Arc<UpdateService>> {
        self.updates.as_ref()
    }

    pub fn restart_hub(&self) -> Option<Arc<RestartHub>> {
        self.hub.clone()
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn server(&self) -> &Arc<ProxyServer> {
        &self.server
    }

    pub fn services(&self) -> &ProxyServices {
        self.server.services()
    }

    pub const fn plugins(&self) -> &PluginManagerHandle {
        &self.plugins
    }

    pub fn console_services(&self) -> ConsoleServices {
        let services = self.services();
        ConsoleServices {
            player_registry: Arc::clone(&services.player_registry),
            connection_registry: Arc::clone(&services.connection_registry),
            ban_manager: Arc::clone(&services.ban_manager),
            server_manager: services.server_manager.clone(),
            config_service: Arc::new(ConfigServiceImpl::new(
                Arc::clone(&services.domain_router),
                services.config_path.clone(),
                Arc::clone(&services.config),
            )),
            plugins: self.plugins.clone(),
            permission_service: Arc::clone(&services.permission_service),
            command_manager: Arc::clone(&services.command_manager),
            shutdown: self.shutdown.clone(),
            start_time: self.start_time,
            updates: self
                .updates
                .clone()
                .map(|updates| updates as Arc<dyn ProxyUpdates>),
            restart: self.hub.as_ref().map(RestartHub::handle),
        }
    }

    pub fn plugin_registry(&self) -> &Arc<PluginRegistryImpl> {
        &self.plugin_registry
    }

    pub fn service_registry(&self) -> &Arc<crate::plugin::service_registry::ServiceRegistryImpl> {
        self.context_factory.service_registry()
    }

    pub async fn disable_plugin(&self, id: &str) -> Result<(), PluginManagerError> {
        self.plugins.disable_plugin(id).await
    }

    pub const fn start_time(&self) -> Instant {
        self.start_time
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }

    pub async fn wait(mut self) -> Result<(), CoreError> {
        if let Some(hub) = &self.hub {
            hub.refuse_requests(WITHOUT_RESTART_LOOP);
        }
        let served = served((&mut self.serve_task).await);
        self.stop(served).await
    }

    pub async fn shutdown(self) -> Result<(), CoreError> {
        self.shutdown.cancel();
        self.wait().await
    }

    pub async fn run_to_exit(mut self) -> Result<ProxyExit, CoreError> {
        let Some(hub) = self.hub.clone() else {
            return self.wait().await.map(|()| ProxyExit::Stopped);
        };
        loop {
            let next = tokio::select! {
                biased;
                joined = &mut self.serve_task => Next::Served(served(joined)),
                () = hub.requested() => Next::Requested,
            };
            let served = match next {
                Next::Served(served) => served,
                Next::Requested => match self.restart(&hub).await {
                    Attempt::Aborted => continue,
                    Attempt::Stop(served) => served,
                    Attempt::Handed(handover) => return Ok(ProxyExit::Restart(handover)),
                    Attempt::Failed(error) => return Err(error),
                },
            };
            return self.stop(served).await.map(|()| ProxyExit::Stopped);
        }
    }

    async fn restart(&mut self, hub: &RestartHub) -> Attempt {
        let requested_at = Instant::now();
        hub.set_phase(RestartPhase::Preflight);
        tracing::info!(
            target_binary = %hub.plan().target.display(),
            "hot restart requested, checking the new binary"
        );
        let checked = tokio::select! {
            biased;
            () = self.shutdown.cancelled() => {
                hub.fail("the proxy is stopping");
                return Attempt::Stop(served((&mut self.serve_task).await));
            }
            checked = self.preflight(hub) => checked,
        };
        match checked {
            Ok(probe) => tracing::info!(
                version = %probe.version,
                "the new binary can take over the sessions"
            ),
            Err(error) => {
                hub.fail(error.to_string());
                return Attempt::Aborted;
            }
        }

        hub.set_phase(RestartPhase::Draining);
        tracing::info!(
            "hot restart: the proxy stops accepting, new connections wait in the kernel queue"
        );
        self.server.request_listener();
        let listener = match (&mut self.serve_task).await {
            Ok(Ok(ServeExit::Restart(listener))) => listener,
            joined => {
                hub.fail("the accept loop ended before handing its listener back");
                return Attempt::Stop(served(joined));
            }
        };
        let relays = Arc::clone(self.server.relays());
        tracing::info!(
            logins = relays.in_flight(),
            grace = ?RESTART_GRACE,
            "hot restart: waiting for the logins in flight"
        );
        let unsettled = tokio::select! {
            biased;
            () = self.shutdown.cancelled() => {
                hub.fail("the proxy is stopping");
                drop(listener);
                return Attempt::Stop(Ok(()));
            }
            left = relays.settled(tokio::time::Instant::now() + RESTART_GRACE) => left,
        };

        hub.set_phase(RestartPhase::Closing);
        tracing::info!(
            unsettled,
            "hot restart: closing the sessions the new image cannot take over"
        );
        self.server.close_sessions(ShutdownKind::Restart);
        self.server.drain_pipeline(RESTART_PIPELINE_DRAIN).await;
        self.stop_plugins().await;
        self.server.stop_background_tasks();

        hub.set_phase(RestartPhase::Freezing);
        let freeze = relays.freeze();
        tracing::info!(
            sessions = relays.active(),
            "hot restart: freezing the forwarding sessions"
        );
        relays.begin_freeze();
        let deadline = tokio::time::Instant::now() + freeze.write_deadline() + FREEZE_MARGIN;
        let unfrozen = relays.drained(deadline).await;
        let lingering = self.server.settle_connections(FROZEN_TASKS).await;
        if unfrozen > 0 || lingering > 0 {
            tracing::warn!(
                unfrozen,
                lingering,
                "hot restart: some connections did not freeze in time and are lost"
            );
        }

        hub.set_phase(RestartPhase::HandingOver);
        let bind = self.services().config.bind;
        let socket = match listener.into_std() {
            Ok(socket) => Some(ListenerHandover { socket, bind }),
            Err(error) => {
                tracing::warn!(%error, "the listener cannot be handed over, the new image binds its own");
                None
            }
        };
        let plan = hub.plan().clone();
        let built = HandoverPackage::build(
            plan.binary.build.version,
            connection_id_watermark(),
            socket,
            relays.take_frozen(),
        );
        let (package, dropped) = match built {
            Ok(built) => built,
            Err(error) => return Attempt::Failed(error.into()),
        };
        for session in &dropped {
            tracing::warn!(
                player = %session.meta.profile.username,
                error = %session.error,
                "a frozen session cannot be handed over"
            );
        }
        tracing::info!(
            frozen = package.state.sessions.len(),
            dropped = dropped.len() + unfrozen,
            elapsed_ms = u64::try_from(requested_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            "hot restart: the handover package is ready"
        );
        Attempt::Handed(Box::new(RestartHandover { package, plan }))
    }

    async fn preflight(
        &self,
        hub: &RestartHub,
    ) -> Result<crate::binary::HandoverProbe, PreflightError> {
        if let Some(manager) = &self.services().server_manager {
            let running = manager.running_child_processes();
            if !running.is_empty() {
                return Err(PreflightError::ChildServers(running));
            }
        }
        hub.preflight().check(hub.plan()).await
    }

    async fn stop(&mut self, served: Result<(), CoreError>) -> Result<(), CoreError> {
        if let Some(hub) = &self.hub {
            hub.mark_stopping();
        }
        self.server.close_sessions(ShutdownKind::Stop);
        self.server.drain_connections(self.drain_timeout).await;
        self.stop_plugins().await;
        self.server.stop_background_tasks();
        served
    }

    async fn stop_plugins(&self) {
        let deadline = tokio::time::Instant::now() + self.plugin_shutdown_timeout;
        let bus = self.server.event_bus();
        let announced = async {
            bus.fire(ProxyShutdownEvent::new()).await;
            bus.flush().await;
        };
        if tokio::time::timeout_at(deadline, announced).await.is_err() {
            tracing::warn!(limit = ?self.plugin_shutdown_timeout,
                "ProxyShutdownEvent listeners used up the plugin shutdown time; stopping the plugins without waiting for them");
        }
        self.plugins.shutdown_until(deadline).await;
    }
}

fn served(
    joined: Result<Result<ServeExit, CoreError>, tokio::task::JoinError>,
) -> Result<(), CoreError> {
    match joined {
        Ok(Ok(ServeExit::Stopped | ServeExit::Restart(_))) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(CoreError::Startup(format!("proxy server task failed: {e}"))),
    }
}

impl Drop for RunningProxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.server.close_sessions(ShutdownKind::Stop);
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
        assert!(running.plugins().is_plugin_loaded("recorder").await);
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

        let ctx = running.plugins().plugin_context("late").await.unwrap();
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

    struct StaticDocuments(&'static str);

    impl infrarust_api::provider::PluginConfigProvider for StaticDocuments {
        fn provider_type(&self) -> &str {
            "static"
        }

        fn load_initial(
            &self,
        ) -> BoxFuture<'_, Result<Vec<infrarust_api::provider::ServerDocument>, PluginError>>
        {
            let document = infrarust_api::provider::ServerDocument {
                id: infrarust_api::types::ServerId::new(self.0),
                toml: format!(
                    "domains = [\"{}.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n",
                    self.0
                ),
            };
            Box::pin(async move { Ok(vec![document]) })
        }

        fn watch(
            &self,
            _sender: Box<dyn infrarust_api::provider::PluginProviderSender>,
        ) -> BoxFuture<'_, Result<(), PluginError>> {
            Box::pin(async { Ok(()) })
        }
    }

    async fn served(ctx: &dyn PluginContext, server: &str) -> bool {
        let id = infrarust_api::types::ServerId::new(server);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if ctx.config_service().get_server_config(&id).is_some() {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_config_provider_registered_after_startup_serves_its_documents() {
        let dir = tempfile::tempdir().unwrap();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let running = ProxyRuntime::builder(test_config(dir.path()), dir.path().join("i.toml"))
            .loader(Box::new(recording_loader("late", &log)))
            .trusted_plugins(["late".to_string()])
            .start()
            .await
            .unwrap();

        let ctx = running.plugins().plugin_context("late").await.unwrap();
        ctx.register_config_provider(Box::new(StaticDocuments("late-lobby")));

        assert!(
            served(ctx.as_ref(), "late-lobby").await,
            "a provider registered after startup must reach the config service"
        );

        running.disable_plugin("late").await.unwrap();
        assert!(
            ctx.config_service()
                .get_server_config(&infrarust_api::types::ServerId::new("late-lobby"))
                .is_none(),
            "disabling the plugin withdraws its documents"
        );
        running.shutdown().await.unwrap();
    }

    struct HangingPlugin;

    impl Plugin for HangingPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new("hanging", "hanging", "1.0.0")
        }

        fn on_enable<'a>(
            &'a self,
            ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            ctx.event_bus()
                .subscribe_async(EventPriority::NORMAL, |_: &mut ProxyShutdownEvent| {
                    Box::pin(tokio::time::sleep(Duration::from_secs(3600)))
                });
            Box::pin(async { Ok(()) })
        }

        fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn a_plugin_hanging_at_shutdown_holds_the_proxy_only_for_the_plugin_shutdown_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let loader = StaticPluginLoader::new();
        loader.register(PluginMetadata::new("hanging", "hanging", "1.0.0"), || {
            Box::new(HangingPlugin)
        });
        let running = ProxyRuntime::builder(test_config(dir.path()), dir.path().join("i.toml"))
            .loader(Box::new(loader))
            .trusted_plugins(["hanging".to_string()])
            .drain_timeout(Duration::from_millis(100))
            .plugin_disable_timeout(Duration::from_millis(300))
            .plugin_shutdown_timeout(Duration::from_millis(800))
            .start()
            .await
            .unwrap();
        let started = Instant::now();
        running.shutdown().await.unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(3),
            "a hanging ProxyShutdownEvent listener and on_disable held the shutdown {took:?}"
        );
    }

    #[tokio::test]
    async fn without_a_binary_no_update_service_runs() {
        let dir = tempfile::tempdir().unwrap();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let running = ProxyRuntime::builder(test_config(dir.path()), dir.path().join("i.toml"))
            .loader(Box::new(recording_loader("probe", &log)))
            .trusted_plugins(["probe".to_string()])
            .start()
            .await
            .unwrap();

        assert!(running.updates().is_none());
        assert!(running.console_services().updates.is_none());
        let ctx = running.plugins().plugin_context("probe").await.unwrap();
        assert!(ctx.proxy_updates().is_none());
        running.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn with_a_binary_the_update_service_reaches_plugins_and_the_console() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("infrarust");
        std::fs::write(&exe, b"binary").unwrap();
        let binary = RunningBinary::new(
            crate::binary::BuildInfo {
                version: "2.0.0-beta.5",
                target: "x86_64-unknown-linux-musl",
                official: true,
                docker: false,
            },
            crate::binary::ExeSnapshot::at(exe),
            Vec::new(),
        );
        let mut config = test_config(dir.path());
        config.update.check = false;
        config.update.url = "http://127.0.0.1:9/releases".to_string();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let running = ProxyRuntime::builder(config, dir.path().join("i.toml"))
            .loader(Box::new(recording_loader("probe", &log)))
            .trusted_plugins(["probe".to_string()])
            .binary(Some(binary))
            .start()
            .await
            .unwrap();

        let service = running
            .updates()
            .expect("a binary brings an update service");
        assert!(!service.config().check);
        assert_eq!(service.status().current_version, "2.0.0-beta.5");
        assert_eq!(service.status().last_check, None);

        let ctx = running.plugins().plugin_context("probe").await.unwrap();
        let handed = ctx.proxy_updates().expect("plugins receive the service");
        assert!(std::ptr::addr_eq(
            Arc::as_ptr(&handed),
            Arc::as_ptr(service)
        ));
        let console = running
            .console_services()
            .updates
            .expect("the console receives the service");
        assert!(std::ptr::addr_eq(
            Arc::as_ptr(&console),
            Arc::as_ptr(service)
        ));
        running.shutdown().await.unwrap();
    }

    fn restartable_config(dir: &std::path::Path) -> ProxyConfig {
        std::fs::create_dir_all(dir.join("servers")).unwrap();
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        let mut config: ProxyConfig = toml::from_str("bind = \"127.0.0.1:0\"\n").unwrap();
        config.servers_dir = dir.join("servers");
        config.plugins_dir = dir.join("plugins");
        config.ban.file = dir.join("bans.json");
        config.update.check = false;
        config
    }

    fn restartable(dir: &std::path::Path, preflight: Arc<dyn Preflight>) -> ProxyRuntimeBuilder {
        ProxyRuntime::builder(restartable_config(dir), dir.join("infrarust.toml"))
            .binary(Some(crate::test_support::test_binary(
                &dir.join("infrarust"),
            )))
            .hot_restart(preflight)
            .drain_timeout(Duration::from_secs(1))
    }

    async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let waited = tokio::time::timeout(Duration::from_secs(5), async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "timed out waiting for {what}");
    }

    #[tokio::test]
    async fn without_a_binary_hot_restart_stays_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let running =
            ProxyRuntime::builder(restartable_config(dir.path()), dir.path().join("i.toml"))
                .hot_restart(crate::test_support::FakePreflight::approving())
                .start()
                .await
                .unwrap();

        assert!(running.restart_hub().is_none());
        assert!(running.console_services().restart.is_none());
        assert!(matches!(
            running.run_to_exit_after_cancel().await.unwrap(),
            ProxyExit::Stopped
        ));
    }

    impl RunningProxy {
        async fn run_to_exit_after_cancel(self) -> Result<ProxyExit, CoreError> {
            self.shutdown.cancel();
            self.run_to_exit().await
        }
    }

    #[tokio::test]
    async fn wait_refuses_restarts_instead_of_leaving_them_pending() {
        let dir = tempfile::tempdir().unwrap();
        let running = restartable(dir.path(), crate::test_support::FakePreflight::approving())
            .start()
            .await
            .unwrap();
        let hub = running.restart_hub().unwrap();
        let shutdown = running.shutdown_token().clone();
        assert!(running.console_services().restart.is_some());

        let waiting = tokio::spawn(running.wait());

        eventually("wait to refuse restarts", || {
            matches!(
                hub.request(),
                Err(infrarust_api::services::proxy_control::RestartRequestError::Unsupported(_))
            )
        })
        .await;
        shutdown.cancel();
        waiting.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_refused_preflight_keeps_the_proxy_serving() {
        let dir = tempfile::tempdir().unwrap();
        let preflight = crate::test_support::FakePreflight::refusing();
        let running = restartable(dir.path(), preflight.clone())
            .start()
            .await
            .unwrap();
        let addr = running.local_addr();
        let hub = running.restart_hub().unwrap();
        let shutdown = running.shutdown_token().clone();
        let run = tokio::spawn(running.run_to_exit());

        hub.request().unwrap();

        eventually("the preflight to fail", || hub.last_failure().is_some()).await;
        assert_eq!(hub.phase(), RestartPhase::Idle);
        assert_eq!(preflight.checks(), 1);
        assert!(
            hub.last_failure()
                .unwrap()
                .contains("cannot take over live sessions")
        );
        tokio::net::TcpStream::connect(addr).await.unwrap();
        hub.request().unwrap();
        eventually("the second preflight", || preflight.checks() == 2).await;

        shutdown.cancel();
        assert!(matches!(run.await.unwrap().unwrap(), ProxyExit::Stopped));
        assert_eq!(
            hub.request(),
            Err(infrarust_api::services::proxy_control::RestartRequestError::ShuttingDown)
        );
    }

    #[tokio::test]
    async fn a_restart_hands_the_listener_over_to_the_successor() {
        let dir = tempfile::tempdir().unwrap();
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let running = restartable(dir.path(), crate::test_support::FakePreflight::approving())
            .loader(Box::new(recording_loader("recorder", &log)))
            .trusted_plugins(["recorder".to_string()])
            .start()
            .await
            .unwrap();
        let addr = running.local_addr();
        let hub = running.restart_hub().unwrap();
        let run = tokio::spawn(running.run_to_exit());

        hub.request().unwrap();

        let exit = tokio::time::timeout(Duration::from_secs(10), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ProxyExit::Restart(handover) = exit else {
            panic!("a requested restart must hand over, got {exit:?}");
        };
        assert_eq!(hub.phase(), RestartPhase::HandingOver);
        assert_eq!(handover.plan.target, dir.path().join("infrarust"));
        let record = handover.package.state.listener.as_ref().unwrap();
        assert_eq!(record.local_addr, addr);
        assert_eq!(record.bind, "127.0.0.1:0".parse().unwrap());
        assert!(handover.package.state.sessions.is_empty());
        assert_eq!(
            *log.lock().unwrap(),
            vec!["enable", "initialize", "shutdown", "disable"]
        );

        let inheritance = crate::hot_restart::MemoryInheritance::new(handover.package);
        let outcome = inheritance.record();
        let successor = restartable(dir.path(), crate::test_support::FakePreflight::approving())
            .loader(Box::new(recording_loader("recorder", &log)))
            .trusted_plugins(["recorder".to_string()])
            .restored(Box::new(inheritance))
            .start()
            .await
            .unwrap();

        assert_eq!(successor.local_addr(), addr);
        assert_eq!(
            outcome.outcome(),
            crate::hot_restart::InheritanceOutcome::Committed
        );
        assert!(tokio::net::TcpStream::connect(addr).await.is_ok());
        assert!(successor.restart_hub().is_some());
        successor.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_successor_that_cannot_unpack_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let listener = ListenerHandover {
            socket,
            bind: "127.0.0.1:0".parse().unwrap(),
        };
        let (mut package, _) =
            HandoverPackage::build("2.0.0-beta.6", 1, Some(listener), Vec::new()).unwrap();
        package.state.listener.as_mut().unwrap().local_addr = "127.0.0.1:1".parse().unwrap();
        let inheritance = crate::hot_restart::MemoryInheritance::new(package);
        let outcome = inheritance.record();

        let started = restartable(dir.path(), crate::test_support::FakePreflight::approving())
            .restored(Box::new(inheritance))
            .start()
            .await;

        let Err(CoreError::Handover(crate::hot_restart::HandoverError::RolledBack(reason))) =
            started
        else {
            panic!("a broken package must roll back");
        };
        assert!(reason.contains("listener bound to"), "{reason}");
        assert_eq!(
            outcome.outcome(),
            crate::hot_restart::InheritanceOutcome::RolledBack(reason)
        );
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

        assert!(!running.plugins().is_plugin_loaded("sleeper").await);
        assert!(matches!(
            running.plugins().plugin_state("sleeper").await,
            Some(PluginState::Disabled)
        ));
        assert!(log.lock().unwrap().is_empty());

        running.shutdown().await.unwrap();
        assert!(log.lock().unwrap().is_empty());
    }
}
