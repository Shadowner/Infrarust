use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::ban_service::BanService;

pub use infrarust_api::loader::PluginContextFactory;

use super::context::PluginContextImpl;
use super::manager::PluginServices;
use crate::ban::BanManager;
use crate::services::ban_bridge::PluginBanService;

/// Per-plugin permissions extracted from proxy configuration.
#[derive(Debug, Clone, Default)]
pub struct PluginPermissions {
    /// Granted capability strings from config (kebab-case, e.g. `["codec-filter", "raw-packet"]`).
    pub permissions: Vec<String>,
    pub deny: Vec<String>,
    pub trusted: bool,
}

pub struct PluginContextFactoryImpl {
    services: PluginServices,
    plugin_configs: HashMap<String, PluginPermissions>,
    contexts: Mutex<HashMap<String, Weak<PluginContextImpl>>>,
    ban_providers: Option<Arc<BanManager>>,
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
            ban_providers: None,
        }
    }

    #[must_use]
    pub fn with_ban_providers(mut self, bans: Arc<BanManager>) -> Self {
        self.ban_providers = Some(bans);
        self
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

        let (capabilities, rejected) = if perms.trusted {
            let mut set = CapabilitySet::native_trusted();
            let unknown = set.revoke_config_strings(&perms.deny);
            (set, unknown)
        } else {
            CapabilitySet::from_config(&perms.permissions, &perms.deny)
        };
        for cap in &rejected {
            tracing::warn!(
                plugin = %plugin_id,
                capability = %cap,
                "ignoring plugin capability: unknown or not grantable via config"
            );
        }

        let ctx = Arc::new(PluginContextImpl::new(
            plugin_id.to_string(),
            Arc::clone(&self.services.event_bus),
            Arc::clone(&self.services.player_registry),
            Arc::clone(&self.services.server_manager),
            Arc::new(PluginBanService::new(
                Arc::clone(&self.services.ban_service),
                plugin_id,
            )) as Arc<dyn BanService>,
            self.ban_providers.clone(),
            Arc::clone(&self.services.config_service),
            Arc::clone(&self.services.load_balancer_service),
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
