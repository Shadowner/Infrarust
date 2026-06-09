use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;

pub use infrarust_api::loader::PluginContextFactory;

use super::context::PluginContextImpl;
use super::manager::PluginServices;

/// Per-plugin permissions extracted from proxy configuration.
#[derive(Debug, Clone, Default)]
pub struct PluginPermissions {
    /// Granted capability strings from config (kebab-case, e.g. `["codec-filter", "raw-packet"]`).
    pub permissions: Vec<String>,
    pub trusted: bool,
}

pub struct PluginContextFactoryImpl {
    services: PluginServices,
    plugin_configs: HashMap<String, PluginPermissions>,
    contexts: Mutex<HashMap<String, Weak<PluginContextImpl>>>,
}

impl PluginContextFactoryImpl {
    pub fn new(
        services: PluginServices,
        plugin_configs: HashMap<String, PluginPermissions>,
    ) -> Self {
        Self {
            services,
            plugin_configs,
            contexts: Mutex::new(HashMap::new()),
        }
    }
}

impl PluginContextFactory for PluginContextFactoryImpl {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        let mut cache = self.contexts.lock().expect("lock poisoned");
        if let Some(existing) = cache.get(plugin_id).and_then(Weak::upgrade) {
            return existing;
        }

        let perms = self
            .plugin_configs
            .get(plugin_id)
            .cloned()
            .unwrap_or_default();

        let capabilities = if perms.trusted {
            CapabilitySet::native_trusted()
        } else {
            let (set, rejected) = CapabilitySet::from_config_strings(&perms.permissions);
            for cap in &rejected {
                tracing::warn!(
                    plugin = %plugin_id,
                    capability = %cap,
                    "ignoring plugin capability: unknown or not grantable via config"
                );
            }
            set
        };

        let ctx = Arc::new(PluginContextImpl::new(
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
            capabilities,
        ));

        cache.insert(plugin_id.to_string(), Arc::downgrade(&ctx));
        ctx
    }

    fn forget_context(&self, plugin_id: &str) {
        self.contexts
            .lock()
            .expect("lock poisoned")
            .remove(plugin_id);
    }
}
