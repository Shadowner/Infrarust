//! [`StaticPluginLoader`] — loads plugins compiled into the binary via Cargo features.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginMetadata};

use super::context_factory::PluginContextFactory;
use super::loader::{LoaderError, PluginLoader};

pub trait PluginFactory: Send + Sync {
    fn metadata(&self) -> PluginMetadata;
    fn create(&self) -> Box<dyn Plugin>;
}

/// A [`PluginFactory`] backed by a closure.
struct FnPluginFactory<F>
where
    F: Fn() -> Box<dyn Plugin> + Send + Sync,
{
    metadata: PluginMetadata,
    factory: F,
}

impl<F> PluginFactory for FnPluginFactory<F>
where
    F: Fn() -> Box<dyn Plugin> + Send + Sync,
{
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }

    fn create(&self) -> Box<dyn Plugin> {
        (self.factory)()
    }
}

/// Plugin loader for statically compiled plugins (Cargo features).
///
/// Plugins are registered explicitly via [`register()`](Self::register).
/// The `plugin_dir` argument in [`discover()`](PluginLoader::discover) is ignored.
pub struct StaticPluginLoader {
    factories: RwLock<HashMap<String, Box<dyn PluginFactory>>>,
}

impl StaticPluginLoader {
    pub fn new() -> Self {
        Self {
            factories: RwLock::new(HashMap::new()),
        }
    }

    /// # Panics
    /// Panics if a plugin with the same ID is already registered.
    pub fn register<F>(&self, metadata: PluginMetadata, factory: F)
    where
        F: Fn() -> Box<dyn Plugin> + Send + Sync + 'static,
    {
        let id = metadata.id.clone();
        let plugin_factory = FnPluginFactory { metadata, factory };

        let mut factories = self.factories.write().expect("lock poisoned");
        if factories.contains_key(&id) {
            panic!("Duplicate static plugin id: {id}");
        }
        factories.insert(id, Box::new(plugin_factory));
    }

    pub fn registered_count(&self) -> usize {
        self.factories.read().expect("lock poisoned").len()
    }
}

impl Default for StaticPluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginLoader for StaticPluginLoader {
    fn name(&self) -> &str {
        "static"
    }

    fn discover<'a>(
        &'a self,
        _plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>> {
        Box::pin(async {
            let factories = self.factories.read().expect("lock poisoned");
            let metadatas = factories.values().map(|f| f.metadata()).collect();
            Ok(metadatas)
        })
    }

    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        _context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
        Box::pin(async move {
            let factories = self.factories.read().expect("lock poisoned");
            let factory = factories
                .get(plugin_id)
                .ok_or_else(|| LoaderError::PluginNotFound {
                    plugin_id: plugin_id.to_string(),
                })?;
            Ok(factory.create())
        })
    }

    fn unload<'a>(&'a self, _plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use infrarust_api::error::PluginError;
    use infrarust_api::plugin::PluginContext;

    use super::*;

    struct TestPlugin {
        id: String,
        enabled: Arc<AtomicBool>,
    }

    impl Plugin for TestPlugin {
        fn metadata(&self) -> PluginMetadata {
            PluginMetadata::new(&self.id, &self.id, "0.1.0")
        }

        fn on_enable<'a>(
            &'a self,
            _ctx: &'a dyn PluginContext,
        ) -> BoxFuture<'a, Result<(), PluginError>> {
            self.enabled.store(true, Ordering::Relaxed);
            Box::pin(async { Ok(()) })
        }

        fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
            self.enabled.store(false, Ordering::Relaxed);
            Box::pin(async { Ok(()) })
        }
    }

    struct MockPluginContext {
        plugin_id: String,
    }

    impl infrarust_api::plugin::private::Sealed for MockPluginContext {}

    impl PluginContext for MockPluginContext {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn event_bus(&self) -> &dyn infrarust_api::event::bus::EventBus {
            unimplemented!("mock")
        }

        fn player_registry(&self) -> &dyn infrarust_api::services::player_registry::PlayerRegistry {
            unimplemented!("mock")
        }

        fn player_registry_handle(
            &self,
        ) -> Arc<dyn infrarust_api::services::player_registry::PlayerRegistry> {
            unimplemented!("mock")
        }

        fn server_manager(&self) -> &dyn infrarust_api::services::server_manager::ServerManager {
            unimplemented!("mock")
        }

        fn ban_service(&self) -> &dyn infrarust_api::services::ban_service::BanService {
            unimplemented!("mock")
        }

        fn config_service(&self) -> &dyn infrarust_api::services::config_service::ConfigService {
            unimplemented!("mock")
        }

        fn command_manager(&self) -> &dyn infrarust_api::command::CommandManager {
            unimplemented!("mock")
        }

        fn scheduler(&self) -> &dyn infrarust_api::services::scheduler::Scheduler {
            unimplemented!("mock")
        }

        fn register_limbo_handler(&self, _handler: Box<dyn infrarust_api::limbo::LimboHandler>) {
            unimplemented!("mock")
        }

        fn register_config_provider(
            &self,
            _provider: Box<dyn infrarust_api::provider::PluginConfigProvider>,
        ) {
            // no-op for tests
        }

        fn codec_filters(
            &self,
        ) -> Option<&dyn infrarust_api::filter::registry::CodecFilterRegistry> {
            None
        }

        fn transport_filters(
            &self,
        ) -> Option<&dyn infrarust_api::filter::registry::TransportFilterRegistry> {
            None
        }

        fn plugin_id(&self) -> &str {
            &self.plugin_id
        }
        fn plugin_registry(&self) -> &dyn infrarust_api::services::plugin_registry::PluginRegistry {
            unimplemented!("mock")
        }
        fn plugin_registry_handle(
            &self,
        ) -> Arc<dyn infrarust_api::services::plugin_registry::PluginRegistry> {
            unimplemented!("mock")
        }
        fn server_manager_handle(
            &self,
        ) -> Arc<dyn infrarust_api::services::server_manager::ServerManager> {
            unimplemented!("mock")
        }
        fn ban_service_handle(&self) -> Arc<dyn infrarust_api::services::ban_service::BanService> {
            unimplemented!("mock")
        }
        fn config_service_handle(
            &self,
        ) -> Arc<dyn infrarust_api::services::config_service::ConfigService> {
            unimplemented!("mock")
        }
        fn event_bus_handle(&self) -> Arc<dyn infrarust_api::event::bus::EventBus> {
            unimplemented!("mock")
        }
        fn proxy_shutdown(&self) -> tokio_util::sync::CancellationToken {
            tokio_util::sync::CancellationToken::new()
        }
    }

    struct MockPluginContextFactory;

    impl PluginContextFactory for MockPluginContextFactory {
        fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
            Arc::new(MockPluginContext {
                plugin_id: plugin_id.to_string(),
            })
        }
    }

    #[tokio::test]
    async fn test_static_loader_discover_returns_registered_plugins() {
        let loader = StaticPluginLoader::new();
        loader.register(PluginMetadata::new("test_a", "Test A", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "test_a".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });
        loader.register(PluginMetadata::new("test_b", "Test B", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "test_b".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });

        let discovered = loader.discover(Path::new("ignored")).await.unwrap();
        assert_eq!(discovered.len(), 2);

        let ids: std::collections::HashSet<String> =
            discovered.iter().map(|m| m.id.clone()).collect();
        assert!(ids.contains("test_a"));
        assert!(ids.contains("test_b"));
    }

    #[tokio::test]
    async fn test_static_loader_load_creates_plugin() {
        let loader = StaticPluginLoader::new();
        let enabled = Arc::new(AtomicBool::new(false));
        let enabled_clone = enabled.clone();

        loader.register(PluginMetadata::new("test", "Test", "1.0.0"), move || {
            Box::new(TestPlugin {
                id: "test".into(),
                enabled: enabled_clone.clone(),
            })
        });

        let mock_factory = MockPluginContextFactory;
        let plugin = loader.load("test", &mock_factory).await.unwrap();
        assert_eq!(plugin.metadata().id, "test");
    }

    #[tokio::test]
    async fn test_static_loader_load_unknown_plugin_returns_error() {
        let loader = StaticPluginLoader::new();
        let mock_factory = MockPluginContextFactory;

        let result = loader.load("nonexistent", &mock_factory).await;
        assert!(matches!(result, Err(LoaderError::PluginNotFound { .. })));
    }

    #[test]
    #[should_panic(expected = "Duplicate static plugin id")]
    fn test_static_loader_duplicate_id_panics() {
        let loader = StaticPluginLoader::new();
        loader.register(PluginMetadata::new("dup", "Dup", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "dup".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });
        loader.register(PluginMetadata::new("dup", "Dup Again", "2.0.0"), || {
            Box::new(TestPlugin {
                id: "dup".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });
    }

    #[tokio::test]
    async fn test_static_loader_unload_is_noop() {
        let loader = StaticPluginLoader::new();
        let result = loader.unload("anything").await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_static_loader_registered_count() {
        let loader = StaticPluginLoader::new();
        assert_eq!(loader.registered_count(), 0);

        loader.register(PluginMetadata::new("a", "A", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "a".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });
        assert_eq!(loader.registered_count(), 1);
    }
}
