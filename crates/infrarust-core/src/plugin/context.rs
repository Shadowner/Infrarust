//! [`PluginContext`] implementation — per-plugin service aggregator.

use std::sync::{Arc, Mutex};

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::limbo::LimboHandler;
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    scheduler::Scheduler, server_manager::ServerManager,
};

/// Per-plugin context that aggregates all proxy services.
///
/// Each plugin receives its own `PluginContextImpl` with shared service
/// references and a unique `plugin_id`.
pub struct PluginContextImpl {
    event_bus: Arc<dyn EventBus>,
    player_registry: Arc<dyn PlayerRegistry>,
    server_manager: Arc<dyn ServerManager>,
    ban_service: Arc<dyn BanService>,
    config_service: Arc<dyn ConfigService>,
    command_manager: Arc<dyn CommandManager>,
    scheduler: Arc<dyn Scheduler>,
    limbo_handlers: Mutex<Vec<Box<dyn LimboHandler>>>,
    plugin_id: String,
}

impl PluginContextImpl {
    /// Creates a new plugin context.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plugin_id: String,
        event_bus: Arc<dyn EventBus>,
        player_registry: Arc<dyn PlayerRegistry>,
        server_manager: Arc<dyn ServerManager>,
        ban_service: Arc<dyn BanService>,
        config_service: Arc<dyn ConfigService>,
        command_manager: Arc<dyn CommandManager>,
        scheduler: Arc<dyn Scheduler>,
    ) -> Self {
        Self {
            event_bus,
            player_registry,
            server_manager,
            ban_service,
            config_service,
            command_manager,
            scheduler,
            limbo_handlers: Mutex::new(Vec::new()),
            plugin_id,
        }
    }

    /// Returns registered limbo handlers (consumed during proxy setup).
    pub fn take_limbo_handlers(&self) -> Vec<Box<dyn LimboHandler>> {
        let mut handlers = self.limbo_handlers.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *handlers)
    }
}

impl infrarust_api::plugin::private::Sealed for PluginContextImpl {}

impl PluginContext for PluginContextImpl {
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

    fn ban_service(&self) -> &dyn BanService {
        self.ban_service.as_ref()
    }

    fn config_service(&self) -> &dyn ConfigService {
        self.config_service.as_ref()
    }

    fn command_manager(&self) -> &dyn CommandManager {
        self.command_manager.as_ref()
    }

    fn scheduler(&self) -> &dyn Scheduler {
        self.scheduler.as_ref()
    }

    fn register_limbo_handler(&self, handler: Box<dyn LimboHandler>) {
        let mut handlers = self.limbo_handlers.lock().unwrap_or_else(|e| e.into_inner());
        handlers.push(handler);
    }

    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}
