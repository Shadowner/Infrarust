use std::collections::HashMap;
use std::sync::Arc;

use infrarust_api::plugin::PluginContext;

pub use infrarust_api::loader::PluginContextFactory;

use super::context::PluginContextImpl;
use super::manager::PluginServices;

/// Per-plugin permissions extracted from proxy configuration.
#[derive(Debug, Clone, Default)]
pub struct PluginPermissions {
    /// Granted permission strings (e.g., `["codec_filter"]`).
    pub permissions: Vec<String>,
}

pub struct PluginContextFactoryImpl {
    services: PluginServices,
    plugin_configs: HashMap<String, PluginPermissions>,
}

impl PluginContextFactoryImpl {
    pub fn new(
        services: PluginServices,
        plugin_configs: HashMap<String, PluginPermissions>,
    ) -> Self {
        Self {
            services,
            plugin_configs,
        }
    }
}

impl PluginContextFactory for PluginContextFactoryImpl {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        let _permissions = self
            .plugin_configs
            .get(plugin_id)
            .cloned()
            .unwrap_or_default();

        Arc::new(PluginContextImpl::new(
            plugin_id.to_string(),
            Arc::clone(&self.services.event_bus),
            Arc::clone(&self.services.player_registry),
            Arc::clone(&self.services.server_manager),
            Arc::clone(&self.services.ban_service),
            Arc::clone(&self.services.config_service),
            Arc::clone(&self.services.plugin_registry),
            Arc::clone(&self.services.command_manager),
            Arc::clone(&self.services.scheduler),
            Arc::clone(&self.services.codec_filter_registry),
            Arc::clone(&self.services.transport_filter_registry),
            Arc::clone(&self.services.domain_router),
            self.services.proxy_shutdown.clone(),
            self.services.proxy_info.clone(),
            self.services.plugins_dir.clone(),
        ))
    }
}
