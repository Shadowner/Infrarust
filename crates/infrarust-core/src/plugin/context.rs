//! [`PluginContext`] implementation — per-plugin service aggregator.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use infrarust_plugin_common::validate_plugin_id;
use tokio_util::sync::CancellationToken;

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use infrarust_api::limbo::{LimboHandler, LimboHandlerError, LimboHandlerRegistration};
use infrarust_api::permissions::{
    Capability, CapabilitySet, PermissionNode, PermissionNodeError, PermissionNodeInfo,
    PermissionProvider,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::provider::PluginConfigProvider;
use infrarust_api::services::providers::{ProviderKind, ProviderRejected};
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::services::scheduler::Scheduler;
use infrarust_api::services::service_registry::ServiceRegistry;
use infrarust_api::services::{
    ban_service::{BanProvider, BanService},
    config_service::ConfigService,
    load_balancer::LoadBalancerService,
    player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry,
    server_manager::ServerManager,
};

use crate::ban::BanManager;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::permissions::PermissionService;
use crate::plugin_messaging::PluginChannels;
use crate::provider::plugin_adapter::PluginProviderActivator;
use crate::services::ban_bridge::PluginBanService;
use crate::services::config_service::ReadOnlyConfigService;
use crate::util::sync::lock;

use super::manager::PluginServices;
use super::service_registry::{PluginServiceRegistry, ServiceRegistryImpl};
use super::tracking::{
    TrackingCodecFilterRegistry, TrackingCommandManager, TrackingEventBus, TrackingScheduler,
    TrackingTransportFilterRegistry,
};

const UNUSABLE_DATA_DIR: &str = ".invalid-plugin-id";

pub struct HostRegistries {
    pub ban_manager: Option<Arc<BanManager>>,
    pub permissions: Arc<PermissionService>,
    pub limbo_handlers: Arc<LimboHandlerRegistry>,
    pub services: Arc<ServiceRegistryImpl>,
    pub channels: PluginChannels,
}

pub struct PluginContextImpl {
    event_bus: Arc<TrackingEventBus>,
    player_registry: Arc<dyn PlayerRegistry>,
    server_manager: Arc<dyn ServerManager>,
    ban_service: Arc<dyn BanService>,
    ban_manager: Option<Arc<BanManager>>,
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
    provider_activator: Arc<PluginProviderActivator>,
    queued_config_providers: Mutex<Vec<Box<dyn PluginConfigProvider>>>,
    codec_filters: Arc<TrackingCodecFilterRegistry>,
    transport_filters: Arc<TrackingTransportFilterRegistry>,
    proxy_shutdown: CancellationToken,
    proxy_info: ProxyInfo,
    plugin_id: String,
    plugins_dir: PathBuf,
    capabilities: CapabilitySet,
    channels: PluginChannels,
}

impl PluginContextImpl {
    pub fn new(
        plugin_id: &str,
        host: &PluginServices,
        registries: HostRegistries,
        capabilities: CapabilitySet,
    ) -> Self {
        let event_bus = Arc::new(TrackingEventBus::new(
            Arc::clone(&host.event_bus),
            plugin_id,
        ));
        let command_manager = Arc::new(TrackingCommandManager::new(
            Arc::clone(&host.command_manager),
            plugin_id.to_owned(),
        ));
        let scheduler = Arc::new(TrackingScheduler::new(
            Arc::clone(&host.scheduler),
            plugin_id,
        ));
        let codec_filters = Arc::new(TrackingCodecFilterRegistry::new(
            Arc::clone(&host.codec_filter_registry),
            plugin_id.to_owned(),
        ));
        let transport_filters = Arc::new(TrackingTransportFilterRegistry::new(
            Arc::clone(&host.transport_filter_registry),
            plugin_id.to_owned(),
        ));
        let services = Arc::new(PluginServiceRegistry::new(registries.services, plugin_id));
        let ban_service: Arc<dyn BanService> = Arc::new(PluginBanService::new(
            Arc::clone(&host.ban_service),
            plugin_id,
        ));

        let config_service: Arc<dyn ConfigService> = if capabilities.has(Capability::ConfigWrite) {
            Arc::clone(&host.config_service)
        } else {
            Arc::new(ReadOnlyConfigService::new(Arc::clone(&host.config_service)))
        };

        Self {
            event_bus,
            player_registry: Arc::clone(&host.player_registry),
            server_manager: Arc::clone(&host.server_manager),
            ban_service,
            ban_manager: registries.ban_manager,
            registered_ban_provider: AtomicBool::new(false),
            permissions: registries.permissions,
            registered_permission_provider: AtomicBool::new(false),
            config_service,
            load_balancer_service: Arc::clone(&host.load_balancer_service),
            plugin_registry: Arc::clone(&host.plugin_registry),
            command_manager,
            scheduler,
            limbo_handlers: registries.limbo_handlers,
            services,
            provider_activator: Arc::clone(&host.provider_activator),
            queued_config_providers: Mutex::new(Vec::new()),
            codec_filters,
            transport_filters,
            proxy_shutdown: host.proxy_shutdown.clone(),
            proxy_info: host.proxy_info.clone(),
            plugin_id: plugin_id.to_owned(),
            plugins_dir: host.plugins_dir.clone(),
            capabilities,
            channels: registries.channels,
        }
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

    pub async fn activate_queued_config_providers(&self) {
        let queued = std::mem::take(&mut *lock(&self.queued_config_providers));
        for provider in queued {
            self.provider_activator
                .activate(&self.plugin_id, provider)
                .await;
        }
    }

    pub fn tracked_commands(&self) -> Vec<String> {
        self.command_manager.tracked()
    }

    pub fn cleanup(&self) {
        let Self {
            event_bus,
            player_registry: _,
            server_manager: _,
            ban_service: _,
            ban_manager,
            registered_ban_provider,
            permissions,
            registered_permission_provider,
            config_service: _,
            load_balancer_service: _,
            plugin_registry: _,
            command_manager,
            scheduler,
            limbo_handlers,
            services,
            provider_activator,
            queued_config_providers,
            codec_filters,
            transport_filters,
            proxy_shutdown: _,
            proxy_info: _,
            plugin_id,
            plugins_dir: _,
            capabilities: _,
            channels,
        } = self;

        event_bus.unsubscribe_all();
        command_manager.unregister_all();
        codec_filters.unregister_all();
        transport_filters.unregister_all();
        scheduler.cancel_all();
        limbo_handlers.unregister_owner(plugin_id);
        services.withdraw_all();

        lock(queued_config_providers).clear();
        provider_activator.deactivate(plugin_id);

        if registered_ban_provider.swap(false, Ordering::SeqCst)
            && let Some(bans) = ban_manager
        {
            bans.unregister_provider(plugin_id);
        }

        if registered_permission_provider.swap(false, Ordering::SeqCst)
            && permissions.unregister_provider(plugin_id)
        {
            self.refresh_online_players();
        }
        permissions.unregister_nodes(plugin_id);
        channels.cleanup();

        tracing::debug!(plugin = %plugin_id, "Plugin resources cleaned up");
    }
}

impl infrarust_api::plugin::private::Sealed for PluginContextImpl {}

impl PluginContext for PluginContextImpl {
    fn event_bus(&self) -> Arc<dyn EventBus> {
        Arc::clone(&self.event_bus) as Arc<dyn EventBus>
    }

    fn player_registry(&self) -> Arc<dyn PlayerRegistry> {
        Arc::clone(&self.player_registry)
    }

    fn server_manager(&self) -> Arc<dyn ServerManager> {
        Arc::clone(&self.server_manager)
    }

    fn ban_service(&self) -> Arc<dyn BanService> {
        Arc::clone(&self.ban_service)
    }

    fn register_ban_provider(
        &self,
        provider: Arc<dyn BanProvider>,
    ) -> Result<(), ProviderRejected> {
        if !self.capabilities.has(Capability::BanProvider) {
            tracing::warn!(
                plugin = %self.plugin_id,
                "register_ban_provider denied: missing ban-provider capability"
            );
            return Err(ProviderRejected::MissingCapability {
                kind: ProviderKind::Ban,
            });
        }
        let Some(bans) = &self.ban_manager else {
            return Err(ProviderRejected::NotSelected {
                kind: ProviderKind::Ban,
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
    ) -> Result<(), ProviderRejected> {
        if !self.capabilities.has(Capability::PermissionProvider) {
            tracing::warn!(
                plugin = %self.plugin_id,
                "register_permission_provider denied: missing permission-provider capability"
            );
            return Err(ProviderRejected::MissingCapability {
                kind: ProviderKind::Permission,
            });
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

    fn config_service(&self) -> Arc<dyn ConfigService> {
        Arc::clone(&self.config_service)
    }

    fn load_balancer_service(&self) -> Arc<dyn LoadBalancerService> {
        Arc::clone(&self.load_balancer_service)
    }

    fn command_manager(&self) -> Arc<dyn CommandManager> {
        Arc::clone(&self.command_manager) as Arc<dyn CommandManager>
    }

    fn scheduler(&self) -> Arc<dyn Scheduler> {
        Arc::clone(&self.scheduler) as Arc<dyn Scheduler>
    }

    fn services(&self) -> Arc<dyn ServiceRegistry> {
        Arc::clone(&self.services) as Arc<dyn ServiceRegistry>
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

    fn plugin_registry(&self) -> Arc<dyn PluginRegistry> {
        Arc::clone(&self.plugin_registry)
    }

    fn register_config_provider(&self, provider: Box<dyn PluginConfigProvider>) {
        let mut queued = lock(&self.queued_config_providers);
        if self.provider_activator.is_started() {
            self.provider_activator.spawn(&self.plugin_id, provider);
        } else {
            queued.push(provider);
        }
    }

    fn codec_filters(&self) -> Option<&dyn CodecFilterRegistry> {
        if self.capabilities.has(Capability::CodecFilter) {
            Some(self.codec_filters.as_ref())
        } else {
            None
        }
    }

    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry> {
        if self.capabilities.has(Capability::TransportFilter) {
            Some(self.transport_filters.as_ref())
        } else {
            None
        }
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn data_dir(&self) -> PathBuf {
        if let Err(invalid) = validate_plugin_id(&self.plugin_id) {
            tracing::error!(
                error = %invalid,
                "No data directory for a plugin whose id breaks the plugin id rule"
            );
            return self.plugins_dir.join(UNUSABLE_DATA_DIR);
        }
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
