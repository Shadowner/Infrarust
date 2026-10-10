//! [`StaticPluginLoader`] — loads plugins compiled into the binary via Cargo features.

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::{Plugin, PluginMetadata};

use super::context_factory::PluginContextFactory;
use super::loader::{LoaderError, PluginLoader};
use crate::util::sync::{read, write};

type PluginConstructor = Box<dyn Fn() -> Box<dyn Plugin> + Send + Sync>;

struct StaticPlugin {
    metadata: PluginMetadata,
    construct: PluginConstructor,
}

#[derive(Default)]
struct Registered {
    in_order: Vec<StaticPlugin>,
    by_id: HashMap<String, usize>,
}

pub struct StaticPluginLoader {
    plugins: RwLock<Registered>,
}

impl StaticPluginLoader {
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(Registered::default()),
        }
    }

    /// # Panics
    /// Panics if a plugin with the same ID is already registered.
    pub fn register<F>(&self, metadata: PluginMetadata, factory: F)
    where
        F: Fn() -> Box<dyn Plugin> + Send + Sync + 'static,
    {
        let id = metadata.id.clone();
        let mut plugins = write(&self.plugins);
        assert!(
            !plugins.by_id.contains_key(&id),
            "Duplicate static plugin id: {id}"
        );
        let index = plugins.in_order.len();
        plugins.by_id.insert(id, index);
        plugins.in_order.push(StaticPlugin {
            metadata,
            construct: Box::new(factory),
        });
    }

    pub fn registered_count(&self) -> usize {
        read(&self.plugins).in_order.len()
    }

    #[must_use]
    pub fn registered_ids(&self) -> Vec<String> {
        read(&self.plugins)
            .in_order
            .iter()
            .map(|plugin| plugin.metadata.id.clone())
            .collect()
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
            Ok(read(&self.plugins)
                .in_order
                .iter()
                .map(|plugin| plugin.metadata.clone())
                .collect())
        })
    }

    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        _context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
        Box::pin(async move {
            let plugins = read(&self.plugins);
            let plugin = plugins
                .by_id
                .get(plugin_id)
                .and_then(|&index| plugins.in_order.get(index))
                .ok_or_else(|| LoaderError::PluginNotFound {
                    plugin_id: plugin_id.to_string(),
                })?;
            Ok((plugin.construct)())
        })
    }

    fn unload<'a>(&'a self, _plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::test_support::{MockPluginContextFactory, TestPlugin};

    use super::*;

    #[tokio::test]
    async fn test_static_loader_discover_returns_registered_plugins() {
        let loader = StaticPluginLoader::new();
        loader.register(PluginMetadata::new("test_a", "Test A", "1.0.0"), || {
            Box::new(TestPlugin::new("test_a"))
        });
        loader.register(PluginMetadata::new("test_b", "Test B", "1.0.0"), || {
            Box::new(TestPlugin::new("test_b"))
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
        loader.register(PluginMetadata::new("test", "Test", "1.0.0"), || {
            Box::new(TestPlugin::new("test"))
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
            Box::new(TestPlugin::new("dup"))
        });
        loader.register(PluginMetadata::new("dup", "Dup Again", "2.0.0"), || {
            Box::new(TestPlugin::new("dup"))
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
            Box::new(TestPlugin::new("a"))
        });
        assert_eq!(loader.registered_count(), 1);
    }
}
