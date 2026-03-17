//! [`PluginManager`] — orchestrates plugin lifecycle.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::command::CommandManager;
use infrarust_api::error::PluginError;
use infrarust_api::event::bus::EventBus;
use infrarust_api::plugin::{Plugin, PluginMetadata};
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    scheduler::Scheduler, server_manager::ServerManager,
};

use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;

use super::PluginState;
use super::context::PluginContextImpl;
use super::dependency::resolve_load_order;

/// Services required to construct per-plugin contexts.
pub struct PluginServices {
    pub event_bus: Arc<dyn EventBus>,
    pub player_registry: Arc<dyn PlayerRegistry>,
    pub server_manager: Arc<dyn ServerManager>,
    pub ban_service: Arc<dyn BanService>,
    pub command_manager: Arc<dyn CommandManager>,
    pub scheduler: Arc<dyn Scheduler>,
    pub config_service: Arc<dyn ConfigService>,
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    pub transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    pub plugins_dir: PathBuf,
}

/// Manages the lifecycle of all loaded plugins.
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
    states: HashMap<String, PluginState>,
}

struct LoadedPlugin {
    plugin: Box<dyn Plugin>,
    context: Option<Arc<PluginContextImpl>>,
    metadata: PluginMetadata,
}

impl PluginManager {
    /// Creates a `PluginManager`, resolving dependencies via topological sort.
    ///
    /// # Errors
    /// Returns `PluginError` if a required dependency is missing or a cycle is detected.
    pub fn new(plugins: Vec<Box<dyn Plugin>>) -> Result<Self, PluginError> {
        if plugins.is_empty() {
            return Ok(Self {
                plugins: Vec::new(),
                states: HashMap::new(),
            });
        }

        // Collect metadata and resolve load order
        let metadata: Vec<PluginMetadata> = plugins.iter().map(|p| p.metadata()).collect();
        let order = resolve_load_order(&metadata)?;

        // Build a lookup from id → index in the original vec
        let mut by_id: HashMap<String, Box<dyn Plugin>> = HashMap::with_capacity(plugins.len());
        for plugin in plugins {
            let id = plugin.metadata().id;
            by_id.insert(id, plugin);
        }

        // Reorder plugins according to topological sort
        let mut ordered = Vec::with_capacity(order.len());
        for id in &order {
            if let Some(plugin) = by_id.remove(id) {
                let meta = plugin.metadata();
                ordered.push(LoadedPlugin {
                    plugin,
                    context: None,
                    metadata: meta,
                });
            }
        }

        Ok(Self {
            plugins: ordered,
            states: HashMap::new(),
        })
    }

    /// Enables all plugins in topological order.
    ///
    /// Each plugin receives a dedicated [`PluginContextImpl`]. If a plugin
    /// fails in `on_enable`, it is marked [`PluginState::Error`] and the
    /// remaining plugins continue loading.
    pub async fn enable_all(&mut self, services: &PluginServices) -> Vec<PluginError> {
        let mut errors = Vec::new();

        for loaded in &mut self.plugins {
            let plugin_id = loaded.metadata.id.clone();
            self.states.insert(plugin_id.clone(), PluginState::Loading);

            let ctx = Arc::new(PluginContextImpl::new(
                plugin_id.clone(),
                Arc::clone(&services.event_bus),
                Arc::clone(&services.player_registry),
                Arc::clone(&services.server_manager),
                Arc::clone(&services.ban_service),
                Arc::clone(&services.config_service),
                Arc::clone(&services.command_manager),
                Arc::clone(&services.scheduler),
                Arc::clone(&services.codec_filter_registry),
                Arc::clone(&services.transport_filter_registry),
            ));

            match loaded.plugin.on_enable(ctx.as_ref()).await {
                Ok(()) => {
                    self.states.insert(plugin_id.clone(), PluginState::Enabled);
                    loaded.context = Some(ctx);
                    tracing::info!(plugin = %plugin_id, "Plugin enabled");
                }
                Err(e) => {
                    self.states
                        .insert(plugin_id.clone(), PluginState::Error(e.to_string()));
                    tracing::error!(plugin = %plugin_id, error = %e, "Plugin failed to enable");
                    ctx.cleanup();
                    errors.push(e);
                }
            }
        }

        errors
    }

    /// Disables all plugins in reverse topological order.
    ///
    /// Calls `on_disable()` then `cleanup()` for each enabled plugin.
    /// Errors in `on_disable()` are logged but do not prevent cleanup.
    pub async fn disable_all(&mut self) {
        for loaded in self.plugins.iter_mut().rev() {
            let state = self.states.get(&loaded.metadata.id);
            if !matches!(state, Some(PluginState::Enabled)) {
                continue;
            }

            tracing::info!(plugin = %loaded.metadata.id, "Disabling plugin");
            self.states
                .insert(loaded.metadata.id.clone(), PluginState::Disabled);

            if let Err(e) = loaded.plugin.on_disable().await {
                tracing::error!(
                    plugin = %loaded.metadata.id,
                    error = %e,
                    "Plugin on_disable() failed"
                );
            }

            // Cleanup is always executed, even if on_disable crashed
            if let Some(ctx) = &loaded.context {
                ctx.cleanup();
            }
        }
    }

    /// Returns `true` if the plugin is loaded and currently enabled.
    pub fn is_plugin_loaded(&self, id: &str) -> bool {
        matches!(self.states.get(id), Some(PluginState::Enabled))
    }

    /// Returns the current state of a plugin.
    pub fn plugin_state(&self, id: &str) -> Option<&PluginState> {
        self.states.get(id)
    }

    /// Lists all plugin metadata in load order.
    pub fn list_plugins(&self) -> Vec<&PluginMetadata> {
        self.plugins.iter().map(|p| &p.metadata).collect()
    }
}
