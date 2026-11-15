use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;

pub use infrarust_api::loader::PluginContextFactory;

use super::context::{HostRegistries, PluginContextImpl};
use super::manager::PluginServices;
use super::service_registry::ServiceRegistryImpl;
use crate::ban::BanManager;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::permissions::PermissionService;
use crate::plugin_messaging::PluginChannels;
use crate::util::sync::lock;

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
    ban_manager: Option<Arc<BanManager>>,
    permissions: Arc<PermissionService>,
    limbo_handlers: Arc<LimboHandlerRegistry>,
    service_registry: Arc<ServiceRegistryImpl>,
    messaging: Option<(
        Arc<crate::plugin_messaging::PluginMessaging>,
        Arc<crate::registry::ConnectionRegistry>,
    )>,
}

impl PluginContextFactoryImpl {
    pub fn new(
        services: PluginServices,
        plugin_configs: HashMap<String, PluginPermissions>,
    ) -> Self {
        let service_registry = Arc::new(ServiceRegistryImpl::new(Some(Arc::clone(
            &services.event_bus,
        ))));
        Self {
            services,
            plugin_configs,
            contexts: Mutex::new(HashMap::new()),
            ban_manager: None,
            permissions: Arc::new(PermissionService::new_sync(&Default::default())),
            limbo_handlers: Arc::new(LimboHandlerRegistry::new()),
            service_registry,
            messaging: None,
        }
    }

    #[must_use]
    pub fn with_messaging(
        mut self,
        messaging: Arc<crate::plugin_messaging::PluginMessaging>,
        players: Arc<crate::registry::ConnectionRegistry>,
    ) -> Self {
        self.messaging = Some((messaging, players));
        self
    }

    #[must_use]
    pub(crate) fn with_limbo_handlers(mut self, registry: Arc<LimboHandlerRegistry>) -> Self {
        self.limbo_handlers = registry;
        self
    }

    pub fn service_registry(&self) -> &Arc<ServiceRegistryImpl> {
        &self.service_registry
    }

    #[must_use]
    pub fn with_ban_manager(mut self, bans: Arc<BanManager>) -> Self {
        self.ban_manager = Some(bans);
        self
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<PermissionService>) -> Self {
        self.permissions = permissions;
        self
    }

    fn registries(&self, plugin_id: &str) -> HostRegistries {
        let channels = match &self.messaging {
            Some((messaging, players)) => {
                PluginChannels::new(plugin_id, messaging, Arc::clone(players))
            }
            None => PluginChannels::default(),
        };
        HostRegistries {
            ban_manager: self.ban_manager.clone(),
            permissions: Arc::clone(&self.permissions),
            limbo_handlers: Arc::clone(&self.limbo_handlers),
            services: Arc::clone(&self.service_registry),
            channels,
        }
    }

    pub fn context(&self, plugin_id: &str) -> Arc<PluginContextImpl> {
        let mut cache = lock(&self.contexts);
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

        let ctx = PluginContextImpl::new(
            plugin_id,
            &self.services,
            self.registries(plugin_id),
            capabilities,
        );
        let ctx = Arc::new(ctx);

        cache.insert(plugin_id.to_string(), Arc::downgrade(&ctx));
        ctx
    }

    pub fn remembers_context(&self, plugin_id: &str) -> bool {
        lock(&self.contexts).contains_key(plugin_id)
    }
}

impl PluginContextFactory for PluginContextFactoryImpl {
    fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
        self.context(plugin_id)
    }

    fn forget_context(&self, plugin_id: &str) {
        lock(&self.contexts).remove(plugin_id);
    }
}
