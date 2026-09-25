//! [`PluginContext`] implementation — per-plugin service aggregator.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use infrarust_api::limbo::{LimboHandler, LimboHandlerError, LimboHandlerRegistration};
use infrarust_api::permissions::{
    Capability, CapabilitySet, PermissionNode, PermissionNodeError, PermissionNodeInfo,
    PermissionProvider, PermissionProviderRejected,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::provider::PluginConfigProvider;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::services::scheduler::Scheduler;
use infrarust_api::services::service_registry::ServiceRegistry;
use infrarust_api::services::{
    ban_service::{BanProvider, BanProviderRejected, BanService},
    config_service::ConfigService,
    load_balancer::LoadBalancerService,
    player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry,
    server_manager::ServerManager,
};

use crate::ban::BanManager;
use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::permissions::PermissionService;
use crate::provider::ProviderId;
use crate::routing::DomainRouter;
use crate::services::command_manager::CommandManagerImpl;
use crate::services::scheduler::SchedulerImpl;

use super::service_registry::{PluginServiceRegistry, ServiceRegistryImpl};
use super::tracking::{TrackingCommandManager, TrackingEventBus, TrackingScheduler};

/// Per-plugin context that aggregates all proxy services.
///
/// Each plugin receives its own `PluginContextImpl` with shared service
/// references and a unique `plugin_id`. Tracking wrappers transparently
/// record all registered listeners, commands, and tasks for automatic
/// cleanup when the plugin is disabled.
pub struct PluginContextImpl {
    event_bus: Arc<TrackingEventBus>,
    player_registry: Arc<dyn PlayerRegistry>,
    server_manager: Arc<dyn ServerManager>,
    ban_service: Arc<dyn BanService>,
    ban_providers: Option<Arc<BanManager>>,
    registered_ban_provider: AtomicBool,
    permissions: Arc<PermissionService>,
    registered_permission_provider: AtomicBool,
    config_service: Arc<dyn ConfigService>,
    load_balancer_service: Arc<dyn LoadBalancerService>,
    plugin_registry: Arc<dyn PluginRegistry>,
    command_manager: Arc<TrackingCommandManager>,
    scheduler: Arc<TrackingScheduler>,
    limbo_handlers: Arc<LimboHandlerRegistry>,
    services: Arc<PluginServiceRegistry>,
    config_providers: Mutex<Vec<Box<dyn PluginConfigProvider>>>,
    codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    domain_router: Arc<DomainRouter>,
    proxy_shutdown: CancellationToken,
    proxy_info: ProxyInfo,
    plugin_id: String,
    plugins_dir: PathBuf,
    capabilities: CapabilitySet,

    registered_provider_ids: Arc<Mutex<Vec<ProviderId>>>,
    registered_provider_tokens: Arc<Mutex<Vec<CancellationToken>>>,
    channels: crate::plugin_messaging::PluginChannels,
}

impl PluginContextImpl {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plugin_id: String,
        event_bus: Arc<EventBusImpl>,
        player_registry: Arc<dyn PlayerRegistry>,
        server_manager: Arc<dyn ServerManager>,
        ban_service: Arc<dyn BanService>,
        ban_providers: Option<Arc<BanManager>>,
        config_service: Arc<dyn ConfigService>,
        load_balancer_service: Arc<dyn LoadBalancerService>,
        plugin_registry: Arc<dyn PluginRegistry>,
        command_manager: Arc<CommandManagerImpl>,
        scheduler: Arc<SchedulerImpl>,
        codec_filter_registry: Arc<CodecFilterRegistryImpl>,
        transport_filter_registry: Arc<TransportFilterRegistryImpl>,
        domain_router: Arc<DomainRouter>,
        proxy_shutdown: CancellationToken,
        proxy_info: ProxyInfo,
        plugins_dir: PathBuf,
        capabilities: CapabilitySet,
    ) -> Self {
        let tracking_bus = Arc::new(TrackingEventBus::new(event_bus, &plugin_id));
        let tracking_cmd = Arc::new(TrackingCommandManager::new(
            command_manager,
            plugin_id.clone(),
        ));
        let tracking_sched = Arc::new(TrackingScheduler::new(scheduler, &plugin_id));
        let services = Arc::new(PluginServiceRegistry::new(
            Arc::new(ServiceRegistryImpl::default()),
            &plugin_id,
        ));

        let config_service: Arc<dyn ConfigService> = if capabilities.has(Capability::ConfigWrite) {
            config_service
        } else {
            Arc::new(crate::services::config_service::ReadOnlyConfigService::new(
                config_service,
            ))
        };

        Self {
            event_bus: tracking_bus,
            player_registry,
            server_manager,
            ban_service,
            ban_providers,
            registered_ban_provider: AtomicBool::new(false),
            permissions: Arc::new(PermissionService::new_sync(&Default::default())),
            registered_permission_provider: AtomicBool::new(false),
            config_service,
            load_balancer_service,
            plugin_registry,
            command_manager: tracking_cmd,
            scheduler: tracking_sched,
            limbo_handlers: Arc::new(LimboHandlerRegistry::new()),
            services,
            config_providers: Mutex::new(Vec::new()),
            codec_filter_registry,
            transport_filter_registry,
            domain_router,
            proxy_shutdown,
            proxy_info,
            plugin_id,
            plugins_dir,
            capabilities,
            registered_provider_ids: Arc::new(Mutex::new(Vec::new())),
            registered_provider_tokens: Arc::new(Mutex::new(Vec::new())),
            channels: crate::plugin_messaging::PluginChannels::default(),
        }
    }

    #[must_use]
    pub fn with_channels(mut self, channels: crate::plugin_messaging::PluginChannels) -> Self {
        self.channels = channels;
        self
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<PermissionService>) -> Self {
        self.permissions = permissions;
        self
    }

    #[must_use]
    pub(crate) fn with_limbo_handlers(mut self, registry: Arc<LimboHandlerRegistry>) -> Self {
        self.limbo_handlers = registry;
        self
    }

    #[must_use]
    pub fn with_services(mut self, registry: Arc<ServiceRegistryImpl>) -> Self {
        self.services = Arc::new(PluginServiceRegistry::new(registry, &self.plugin_id));
        self
    }

    pub fn tracked_tasks(&self) -> usize {
        self.scheduler.tracked_count()
    }

    fn refresh_online_players(&self) {
        let players = self.player_registry.get_all_players();
        if players.is_empty() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            for player in players {
                player.refresh_permissions().await;
            }
        });
    }

    pub fn limbo_handlers(&self) -> Vec<Arc<dyn LimboHandler>> {
        self.limbo_handlers.owned_by(&self.plugin_id)
    }

    pub fn take_config_providers(&self) -> Vec<Box<dyn PluginConfigProvider>> {
        let mut providers = self.config_providers.lock().expect("lock poisoned");
        std::mem::take(&mut *providers)
    }

    pub fn register_active_provider_ids(&self, ids: Vec<ProviderId>) {
        self.registered_provider_ids
            .lock()
            .expect("lock poisoned")
            .extend(ids);
    }

    pub fn register_provider_token(&self, token: CancellationToken) {
        self.registered_provider_tokens
            .lock()
            .expect("lock poisoned")
            .push(token);
    }

    pub fn tracked_commands(&self) -> Vec<String> {
        self.command_manager.tracked()
    }

    pub fn cleanup(&self) {
        // Unsubscribe all event listeners
        self.event_bus.unsubscribe_all();

        // Unregister all commands
        self.command_manager.unregister_all();

        self.scheduler.cancel_all();
        self.limbo_handlers.unregister_owner(&self.plugin_id);
        self.services.withdraw_all();

        let tokens = std::mem::take(
            &mut *self
                .registered_provider_tokens
                .lock()
                .expect("lock poisoned"),
        );
        for token in tokens {
            token.cancel();
        }

        let provider_ids =
            std::mem::take(&mut *self.registered_provider_ids.lock().expect("lock poisoned"));
        for pid in &provider_ids {
            self.domain_router.remove(pid);
        }

        if self.registered_ban_provider.swap(false, Ordering::SeqCst)
            && let Some(bans) = &self.ban_providers
        {
            bans.unregister_provider(&self.plugin_id);
        }

        if self
            .registered_permission_provider
            .swap(false, Ordering::SeqCst)
            && self.permissions.unregister_provider(&self.plugin_id)
        {
            self.refresh_online_players();
        }
        self.permissions.unregister_nodes(&self.plugin_id);
        self.channels.cleanup();

        tracing::debug!(plugin = %self.plugin_id, "Plugin resources cleaned up");
    }
}

impl infrarust_api::plugin::private::Sealed for PluginContextImpl {}

impl PluginContext for PluginContextImpl {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn event_bus(&self) -> &dyn EventBus {
        self.event_bus.as_ref()
    }

    fn player_registry(&self) -> &dyn PlayerRegistry {
        self.player_registry.as_ref()
    }

    fn player_registry_handle(&self) -> Arc<dyn PlayerRegistry> {
        Arc::clone(&self.player_registry)
    }

    fn server_manager(&self) -> &dyn ServerManager {
        self.server_manager.as_ref()
    }

    fn server_manager_handle(&self) -> Arc<dyn ServerManager> {
        Arc::clone(&self.server_manager)
    }

    fn ban_service(&self) -> &dyn BanService {
        self.ban_service.as_ref()
    }

    fn ban_service_handle(&self) -> Arc<dyn BanService> {
        Arc::clone(&self.ban_service)
    }

    fn register_ban_provider(
        &self,
        provider: Arc<dyn BanProvider>,
    ) -> Result<(), BanProviderRejected> {
        if !self.capabilities.has(Capability::BanProvider) {
            tracing::warn!(
                plugin = %self.plugin_id,
                "register_ban_provider denied: missing ban-provider capability"
            );
            return Err(BanProviderRejected::MissingCapability);
        }
        let Some(bans) = &self.ban_providers else {
            return Err(BanProviderRejected::NotSelected {
                selected: infrarust_config::BanProviderSelection::BUILTIN.to_string(),
            });
        };
        bans.register_provider(&self.plugin_id, provider)?;
        self.registered_ban_provider.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn register_permission_provider(
        &self,
        provider: Arc<dyn PermissionProvider>,
    ) -> Result<(), PermissionProviderRejected> {
        if !self.capabilities.has(Capability::PermissionProvider) {
            tracing::warn!(
                plugin = %self.plugin_id,
                "register_permission_provider denied: missing permission-provider capability"
            );
            return Err(PermissionProviderRejected::MissingCapability);
        }
        self.permissions
            .register_provider(&self.plugin_id, provider)?;
        self.registered_permission_provider
            .store(true, Ordering::SeqCst);
        self.refresh_online_players();
        Ok(())
    }

    fn register_permission_node(&self, node: PermissionNode) -> Result<(), PermissionNodeError> {
        self.permissions.register_node(Some(&self.plugin_id), node)
    }

    fn permission_nodes(&self) -> Vec<PermissionNodeInfo> {
        self.permissions.nodes()
    }

    fn config_service(&self) -> &dyn ConfigService {
        self.config_service.as_ref()
    }

    fn config_service_handle(&self) -> Arc<dyn ConfigService> {
        Arc::clone(&self.config_service)
    }

    fn load_balancer_service(&self) -> &dyn LoadBalancerService {
        self.load_balancer_service.as_ref()
    }

    fn load_balancer_service_handle(&self) -> Arc<dyn LoadBalancerService> {
        Arc::clone(&self.load_balancer_service)
    }

    fn command_manager(&self) -> &dyn CommandManager {
        self.command_manager.as_ref()
    }

    fn command_manager_handle(&self) -> Arc<dyn CommandManager> {
        Arc::clone(&self.command_manager) as Arc<dyn CommandManager>
    }

    fn scheduler(&self) -> &dyn Scheduler {
        self.scheduler.as_ref()
    }

    fn scheduler_handle(&self) -> Arc<dyn Scheduler> {
        Arc::clone(&self.scheduler) as Arc<dyn Scheduler>
    }

    fn services(&self) -> &dyn ServiceRegistry {
        self.services.as_ref()
    }

    fn services_handle(&self) -> Arc<dyn ServiceRegistry> {
        Arc::clone(&self.services) as Arc<dyn ServiceRegistry>
    }

    fn event_bus_handle(&self) -> Arc<dyn EventBus> {
        Arc::clone(&self.event_bus) as Arc<dyn EventBus>
    }

    fn register_limbo_handler(
        &self,
        handler: Box<dyn LimboHandler>,
    ) -> Result<LimboHandlerRegistration, LimboHandlerError> {
        if !self.capabilities.has(Capability::Limbo) {
            tracing::warn!(
                plugin = %self.plugin_id,
                "register_limbo_handler denied: missing Limbo capability"
            );
            return Err(LimboHandlerError::MissingCapability);
        }
        let name = handler.name().to_string();
        let id = self
            .limbo_handlers
            .register(&self.plugin_id, handler)
            .inspect_err(|e| tracing::warn!(plugin = %self.plugin_id, "{e}"))?;
        let registry = Arc::downgrade(&self.limbo_handlers);
        Ok(LimboHandlerRegistration::new(name, move || {
            registry
                .upgrade()
                .is_some_and(|registry| registry.unregister(id))
        }))
    }

    fn plugin_registry(&self) -> &dyn PluginRegistry {
        self.plugin_registry.as_ref()
    }

    fn plugin_registry_handle(&self) -> Arc<dyn PluginRegistry> {
        Arc::clone(&self.plugin_registry)
    }

    fn register_config_provider(&self, provider: Box<dyn PluginConfigProvider>) {
        let mut providers = self.config_providers.lock().expect("lock poisoned");
        providers.push(provider);
    }

    fn codec_filters(&self) -> Option<&dyn CodecFilterRegistry> {
        if self.capabilities.has(Capability::CodecFilter) {
            Some(self.codec_filter_registry.as_ref())
        } else {
            None
        }
    }

    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry> {
        if self.capabilities.has(Capability::TransportFilter) {
            Some(self.transport_filter_registry.as_ref())
        } else {
            None
        }
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn data_dir(&self) -> PathBuf {
        let dir = self.plugins_dir.join(&self.plugin_id);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                plugin = %self.plugin_id,
                path = %dir.display(),
                error = %e,
                "Failed to create the plugin data directory"
            );
        }

        dir
    }

    fn proxy_shutdown(&self) -> CancellationToken {
        self.proxy_shutdown.clone()
    }

    fn proxy_info(&self) -> &ProxyInfo {
        &self.proxy_info
    }

    fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    fn channel_registrar(&self) -> &dyn infrarust_api::messaging::ChannelRegistrar {
        self.channels.registrar()
    }

    fn server_messenger(&self) -> Arc<dyn infrarust_api::messaging::ServerMessenger> {
        self.channels.messenger()
    }
}
