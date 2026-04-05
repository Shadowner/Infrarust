//! [`PluginContext`] implementation — per-plugin service aggregator.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use infrarust_api::command::CommandManager;
use infrarust_api::event::ListenerHandle;
use infrarust_api::event::bus::EventBus;
use infrarust_api::filter::registry::{CodecFilterRegistry, TransportFilterRegistry};
use infrarust_api::limbo::LimboHandler;
use infrarust_api::plugin::PluginContext;
use infrarust_api::provider::PluginConfigProvider;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::services::scheduler::{Scheduler, TaskHandle};
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry, server_manager::ServerManager,
};

use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::provider::ProviderId;
use crate::routing::DomainRouter;

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
    config_service: Arc<dyn ConfigService>,
    plugin_registry: Arc<dyn PluginRegistry>,
    command_manager: Arc<TrackingCommandManager>,
    scheduler: Arc<TrackingScheduler>,
    limbo_handlers: Mutex<Vec<Box<dyn LimboHandler>>>,
    config_providers: Mutex<Vec<Box<dyn PluginConfigProvider>>>,
    codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    domain_router: Arc<DomainRouter>,
    proxy_shutdown: CancellationToken,
    proxy_info: ProxyInfo,
    plugin_id: String,
    plugins_dir: PathBuf,

    // Shared tracking state (also held by the wrappers)
    registered_handles: Arc<Mutex<Vec<ListenerHandle>>>,
    registered_commands: Arc<Mutex<Vec<String>>>,
    registered_tasks: Arc<Mutex<Vec<TaskHandle>>>,
    registered_provider_ids: Arc<Mutex<Vec<ProviderId>>>,
    registered_provider_tokens: Arc<Mutex<Vec<CancellationToken>>>,
}

impl PluginContextImpl {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plugin_id: String,
        event_bus: Arc<dyn EventBus>,
        player_registry: Arc<dyn PlayerRegistry>,
        server_manager: Arc<dyn ServerManager>,
        ban_service: Arc<dyn BanService>,
        config_service: Arc<dyn ConfigService>,
        plugin_registry: Arc<dyn PluginRegistry>,
        command_manager: Arc<dyn CommandManager>,
        scheduler: Arc<dyn Scheduler>,
        codec_filter_registry: Arc<CodecFilterRegistryImpl>,
        transport_filter_registry: Arc<TransportFilterRegistryImpl>,
        domain_router: Arc<DomainRouter>,
        proxy_shutdown: CancellationToken,
        proxy_info: ProxyInfo,
        plugins_dir: PathBuf,
    ) -> Self {
        let registered_handles = Arc::new(Mutex::new(Vec::new()));
        let registered_commands = Arc::new(Mutex::new(Vec::new()));
        let registered_tasks = Arc::new(Mutex::new(Vec::new()));

        let tracking_bus = Arc::new(TrackingEventBus::new(
            event_bus,
            Arc::clone(&registered_handles),
        ));
        let tracking_cmd = Arc::new(TrackingCommandManager::new(
            command_manager,
            Arc::clone(&registered_commands),
            plugin_id.clone(),
        ));
        let tracking_sched = Arc::new(TrackingScheduler::new(
            scheduler,
            Arc::clone(&registered_tasks),
        ));

        Self {
            event_bus: tracking_bus,
            player_registry,
            server_manager,
            ban_service,
            config_service,
            plugin_registry,
            command_manager: tracking_cmd,
            scheduler: tracking_sched,
            limbo_handlers: Mutex::new(Vec::new()),
            config_providers: Mutex::new(Vec::new()),
            codec_filter_registry,
            transport_filter_registry,
            domain_router,
            proxy_shutdown,
            proxy_info,
            plugin_id,
            plugins_dir,
            registered_handles,
            registered_commands,
            registered_tasks,
            registered_provider_ids: Arc::new(Mutex::new(Vec::new())),
            registered_provider_tokens: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns registered limbo handlers (consumed during proxy setup).
    pub fn take_limbo_handlers(&self) -> Vec<Box<dyn LimboHandler>> {
        let mut handlers = self.limbo_handlers.lock().expect("lock poisoned");
        std::mem::take(&mut *handlers)
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

    pub fn cleanup(&self) {
        // Unsubscribe all event listeners
        let handles = std::mem::take(&mut *self.registered_handles.lock().expect("lock poisoned"));
        for handle in handles {
            self.event_bus.unsubscribe(handle);
        }

        // Unregister all commands
        let commands =
            std::mem::take(&mut *self.registered_commands.lock().expect("lock poisoned"));
        for cmd in commands {
            self.command_manager.unregister(&cmd);
        }

        // Cancel all scheduled tasks
        let tasks = std::mem::take(&mut *self.registered_tasks.lock().expect("lock poisoned"));
        for task in tasks {
            self.scheduler.cancel(task);
        }

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

    fn config_service(&self) -> &dyn ConfigService {
        self.config_service.as_ref()
    }

    fn config_service_handle(&self) -> Arc<dyn ConfigService> {
        Arc::clone(&self.config_service)
    }

    fn command_manager(&self) -> &dyn CommandManager {
        self.command_manager.as_ref()
    }

    fn scheduler(&self) -> &dyn Scheduler {
        self.scheduler.as_ref()
    }

    fn event_bus_handle(&self) -> Arc<dyn EventBus> {
        Arc::clone(&self.event_bus) as Arc<dyn EventBus>
    }

    fn register_limbo_handler(&self, handler: Box<dyn LimboHandler>) {
        let mut handlers = self.limbo_handlers.lock().expect("lock poisoned");
        handlers.push(handler);
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
        Some(self.codec_filter_registry.as_ref())
    }

    fn transport_filters(&self) -> Option<&dyn TransportFilterRegistry> {
        Some(self.transport_filter_registry.as_ref())
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn data_dir(&self) -> PathBuf {
        self.plugins_dir.join(&self.plugin_id)
    }

    fn proxy_shutdown(&self) -> CancellationToken {
        self.proxy_shutdown.clone()
    }

    fn proxy_info(&self) -> &ProxyInfo {
        &self.proxy_info
    }
}
