use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::{ProxyRestart, ProxyUpdates};

pub use infrarust_api::loader::PluginContextFactory;

use super::context::{HostRegistries, PluginContextImpl};
use super::manager::PluginServices;
use super::service_registry::ServiceRegistryImpl;
use crate::ban::BanManager;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::permissions::PermissionService;
use crate::plugin_messaging::{PluginChannels, PluginMessaging};
use crate::registry::ConnectionRegistry;
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
    messaging: Option<Messaging>,
    updates: Option<Arc<dyn ProxyUpdates>>,
    restart: Option<Arc<dyn ProxyRestart>>,
}

struct Messaging {
    channels: Arc<PluginMessaging>,
    players: Arc<ConnectionRegistry>,
}

pub(crate) struct HostWiring {
    pub(crate) ban_manager: Arc<BanManager>,
    pub(crate) permissions: Arc<PermissionService>,
    pub(crate) limbo_handlers: Arc<LimboHandlerRegistry>,
    pub(crate) channels: Arc<PluginMessaging>,
    pub(crate) players: Arc<ConnectionRegistry>,
    pub(crate) updates: Option<Arc<dyn ProxyUpdates>>,
    pub(crate) restart: Option<Arc<dyn ProxyRestart>>,
}

impl PluginContextFactoryImpl {
    pub(crate) fn new(
        services: PluginServices,
        plugin_configs: HashMap<String, PluginPermissions>,
        wiring: HostWiring,
    ) -> Self {
        let HostWiring {
            ban_manager,
            permissions,
            limbo_handlers,
            channels,
            players,
            updates,
            restart,
        } = wiring;
        let service_registry = Arc::new(ServiceRegistryImpl::new(Some(Arc::clone(
            &services.event_bus,
        ))));
        Self {
            services,
            plugin_configs,
            contexts: Mutex::new(HashMap::new()),
            ban_manager: Some(ban_manager),
            permissions,
            limbo_handlers,
            service_registry,
            messaging: Some(Messaging { channels, players }),
            updates,
            restart,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(
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
            updates: None,
            restart: None,
        }
    }

    pub fn service_registry(&self) -> &Arc<ServiceRegistryImpl> {
        &self.service_registry
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_ban_manager(mut self, bans: Arc<BanManager>) -> Self {
        self.ban_manager = Some(bans);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<PermissionService>) -> Self {
        self.permissions = permissions;
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_updates(mut self, updates: Arc<dyn ProxyUpdates>) -> Self {
        self.updates = Some(updates);
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_restart(mut self, restart: Arc<dyn ProxyRestart>) -> Self {
        self.restart = Some(restart);
        self
    }

    fn registries(&self, plugin_id: &str) -> HostRegistries {
        let channels = match &self.messaging {
            Some(Messaging { channels, players }) => {
                PluginChannels::new(plugin_id, channels, Arc::clone(players))
            }
            None => PluginChannels::default(),
        };
        HostRegistries {
            ban_manager: self.ban_manager.clone(),
            permissions: Arc::clone(&self.permissions),
            limbo_handlers: Arc::clone(&self.limbo_handlers),
            services: Arc::clone(&self.service_registry),
            channels,
            updates: self.updates.clone(),
            restart: self.restart.clone(),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::test_support::FakeUpdates;

    #[test]
    fn plugin_contexts_hand_out_the_wired_update_service() {
        let updates: Arc<dyn ProxyUpdates> =
            Arc::new(FakeUpdates::new(FakeUpdates::idle("2.0.0-beta.6")));
        let factory =
            PluginContextFactoryImpl::for_tests(PluginServices::for_tests(), HashMap::new())
                .with_updates(Arc::clone(&updates));

        let ctx = factory.create_context("admin_api");
        let handed = ctx
            .proxy_updates()
            .expect("the context carries the service");
        assert!(Arc::ptr_eq(&handed, &updates));
        assert_eq!(handed.status().current_version, "2.0.0-beta.6");
    }

    #[test]
    fn plugin_contexts_have_no_update_service_unless_one_is_wired() {
        let factory =
            PluginContextFactoryImpl::for_tests(PluginServices::for_tests(), HashMap::new());
        assert!(
            factory
                .create_context("admin_api")
                .proxy_updates()
                .is_none()
        );
    }

    struct CountingRestart(std::sync::atomic::AtomicUsize);

    impl infrarust_api::__sealed::Sealed for CountingRestart {}

    impl ProxyRestart for CountingRestart {
        fn request_restart_with(
            &self,
            _options: infrarust_api::services::proxy_control::RestartOptions,
        ) -> Result<(), infrarust_api::services::proxy_control::RestartRequestError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn plugin_contexts_hand_out_the_wired_restart_handle_only() {
        let restart = Arc::new(CountingRestart(std::sync::atomic::AtomicUsize::new(0)));
        let handle: Arc<dyn ProxyRestart> = restart.clone();
        let wired =
            PluginContextFactoryImpl::for_tests(PluginServices::for_tests(), HashMap::new())
                .with_restart(handle);
        let bare = PluginContextFactoryImpl::for_tests(PluginServices::for_tests(), HashMap::new());

        wired
            .create_context("admin_api")
            .proxy_restart()
            .expect("the context carries the handle")
            .request_restart()
            .unwrap();

        assert_eq!(restart.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(bare.create_context("admin_api").proxy_restart().is_none());
    }
}
