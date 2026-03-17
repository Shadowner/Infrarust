//! [`PluginContext`] implementation — per-plugin service aggregator.

use std::sync::{Arc, Mutex};

use infrarust_api::command::CommandManager;
use infrarust_api::event::bus::EventBus;
use infrarust_api::event::ListenerHandle;
use infrarust_api::limbo::LimboHandler;
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::scheduler::{Scheduler, TaskHandle};
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    server_manager::ServerManager,
};

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
    command_manager: Arc<TrackingCommandManager>,
    scheduler: Arc<TrackingScheduler>,
    limbo_handlers: Mutex<Vec<Box<dyn LimboHandler>>>,
    plugin_id: String,

    // Shared tracking state (also held by the wrappers)
    registered_handles: Arc<Mutex<Vec<ListenerHandle>>>,
    registered_commands: Arc<Mutex<Vec<String>>>,
    registered_tasks: Arc<Mutex<Vec<TaskHandle>>>,
}

impl PluginContextImpl {
    /// Creates a new plugin context with tracking wrappers.
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
        let registered_handles = Arc::new(Mutex::new(Vec::new()));
        let registered_commands = Arc::new(Mutex::new(Vec::new()));
        let registered_tasks = Arc::new(Mutex::new(Vec::new()));

        let tracking_bus =
            Arc::new(TrackingEventBus::new(event_bus, Arc::clone(&registered_handles)));
        let tracking_cmd = Arc::new(TrackingCommandManager::new(
            command_manager,
            Arc::clone(&registered_commands),
        ));
        let tracking_sched =
            Arc::new(TrackingScheduler::new(scheduler, Arc::clone(&registered_tasks)));

        Self {
            event_bus: tracking_bus,
            player_registry,
            server_manager,
            ban_service,
            config_service,
            command_manager: tracking_cmd,
            scheduler: tracking_sched,
            limbo_handlers: Mutex::new(Vec::new()),
            plugin_id,
            registered_handles,
            registered_commands,
            registered_tasks,
        }
    }

    /// Returns registered limbo handlers (consumed during proxy setup).
    pub fn take_limbo_handlers(&self) -> Vec<Box<dyn LimboHandler>> {
        let mut handlers = self.limbo_handlers.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *handlers)
    }

    /// Removes all listeners, commands, and scheduled tasks registered by this plugin.
    pub fn cleanup(&self) {
        // Unsubscribe all event listeners
        let handles = std::mem::take(
            &mut *self
                .registered_handles
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for handle in handles {
            self.event_bus.unsubscribe(handle);
        }

        // Unregister all commands
        let commands = std::mem::take(
            &mut *self
                .registered_commands
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for cmd in commands {
            self.command_manager.unregister(&cmd);
        }

        // Cancel all scheduled tasks
        let tasks = std::mem::take(
            &mut *self
                .registered_tasks
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for task in tasks {
            self.scheduler.cancel(task);
        }

        tracing::debug!(plugin = %self.plugin_id, "Plugin resources cleaned up");
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
