//! [`PluginManager`] — orchestrates plugin lifecycle.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use infrarust_api::command::CommandManager;
use infrarust_api::error::PluginError;
use infrarust_api::event::bus::EventBus;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, player_registry::PlayerRegistry,
    plugin_registry::PluginRegistry, proxy_info::ProxyInfo, scheduler::Scheduler,
    server_manager::ServerManager,
};
use tokio_util::sync::CancellationToken;

use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;

use super::PluginState;
use super::context::PluginContextImpl;
use super::context_factory::PluginContextFactory;
use super::dependency::resolve_load_order;
use super::loader::PluginLoader;

/// Services required to construct per-plugin contexts.
pub struct PluginServices {
    pub event_bus: Arc<dyn EventBus>,
    pub player_registry: Arc<dyn PlayerRegistry>,
    pub server_manager: Arc<dyn ServerManager>,
    pub ban_service: Arc<dyn BanService>,
    pub command_manager: Arc<dyn CommandManager>,
    pub scheduler: Arc<dyn Scheduler>,
    pub config_service: Arc<dyn ConfigService>,
    pub plugin_registry: Arc<dyn PluginRegistry>,
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    pub transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    pub domain_router: Arc<crate::routing::DomainRouter>,
    pub proxy_shutdown: CancellationToken,
    pub proxy_info: ProxyInfo,
    pub plugins_dir: PathBuf,
}

pub struct PluginManager {
    loaders: Vec<Box<dyn PluginLoader>>,
    plugins: Vec<LoadedPlugin>,
    states: HashMap<String, PluginState>,
    load_order: Vec<String>,
    loader_mapping: HashMap<String, String>, // plugin_id -> loader_name
    loaded_loaders: Vec<String>,             // loaders whose on_load() succeeded
}

struct LoadedPlugin {
    plugin: Box<dyn Plugin>,
    context: Arc<dyn PluginContext>,
    metadata: PluginMetadata,
    loader_name: String,
}

impl PluginManager {
    pub fn new(loaders: Vec<Box<dyn PluginLoader>>) -> Self {
        Self {
            loaders,
            plugins: Vec::new(),
            states: HashMap::new(),
            load_order: Vec::new(),
            loader_mapping: HashMap::new(),
            loaded_loaders: Vec::new(),
        }
    }

    /// Discovers all plugins via loaders, detects duplicate IDs,
    /// and resolves load order via topological sort.
    pub async fn discover_all(
        &mut self,
        plugin_dir: &Path,
    ) -> Result<Vec<PluginMetadata>, PluginError> {
        let mut all_metadata: Vec<PluginMetadata> = Vec::new();
        let mut seen_ids: HashMap<String, String> = HashMap::new();

        for loader in &self.loaders {
            let discovered = loader.discover(plugin_dir).await.map_err(|e| {
                PluginError::InitFailed(format!("Loader '{}' discovery failed: {e}", loader.name()))
            })?;

            for metadata in discovered {
                if let Some(existing_loader) = seen_ids.get(&metadata.id) {
                    return Err(PluginError::InitFailed(format!(
                        "Duplicate plugin id '{}': found in loader '{}' and '{}'",
                        metadata.id,
                        existing_loader,
                        loader.name()
                    )));
                }
                seen_ids.insert(metadata.id.clone(), loader.name().to_string());
                all_metadata.push(metadata);
            }
        }

        let load_order = resolve_load_order(&all_metadata)?;

        self.load_order = load_order;
        self.loader_mapping = seen_ids;

        Ok(all_metadata)
    }

    pub async fn load_and_enable_all(
        &mut self,
        context_factory: &dyn PluginContextFactory,
    ) -> Vec<PluginError> {
        let mut errors = Vec::new();
        let load_order = self.load_order.clone();

        let mut failed_loaders: HashSet<String> = HashSet::new();
        let mut ok_loaders: Vec<String> = Vec::new();
        for loader in &self.loaders {
            match loader.on_load(context_factory).await {
                Ok(()) => ok_loaders.push(loader.name().to_string()),
                Err(e) => {
                    let name = loader.name().to_string();
                    tracing::error!(loader = %name, error = %e, "Loader on_load() failed");
                    errors.push(PluginError::InitFailed(format!(
                        "Loader '{name}' on_load failed: {e}"
                    )));
                    failed_loaders.insert(name);
                }
            }
        }
        self.loaded_loaders = ok_loaders;

        for plugin_id in &load_order {
            let loader_name = match self.loader_mapping.get(plugin_id) {
                Some(name) => name.clone(),
                None => {
                    errors.push(PluginError::InitFailed(format!(
                        "No loader mapping for plugin '{plugin_id}'"
                    )));
                    continue;
                }
            };

            if failed_loaders.contains(&loader_name) {
                self.states.insert(
                    plugin_id.clone(),
                    PluginState::Error(format!("loader '{loader_name}' on_load failed")),
                );
                continue;
            }

            let loader = match self.loaders.iter().find(|l| l.name() == loader_name) {
                Some(l) => l,
                None => {
                    errors.push(PluginError::InitFailed(format!(
                        "Loader '{loader_name}' not found for plugin '{plugin_id}'"
                    )));
                    continue;
                }
            };

            let plugin = match loader.load(plugin_id, context_factory).await {
                Ok(p) => p,
                Err(e) => {
                    let err = PluginError::InitFailed(format!(
                        "Loader '{loader_name}' failed to load '{plugin_id}': {e}"
                    ));
                    self.states
                        .insert(plugin_id.clone(), PluginState::Error(e.to_string()));
                    tracing::error!(plugin = %plugin_id, error = %e, "Plugin failed to load");
                    errors.push(err);
                    continue;
                }
            };

            let metadata = plugin.metadata();
            let ctx = context_factory.create_context(plugin_id);

            self.states.insert(plugin_id.clone(), PluginState::Loading);
            match plugin.on_enable(ctx.as_ref()).await {
                Ok(()) => {
                    self.states.insert(plugin_id.clone(), PluginState::Enabled);
                    tracing::info!(plugin = %plugin_id, "Plugin enabled");

                    self.plugins.push(LoadedPlugin {
                        plugin,
                        context: ctx,
                        metadata,
                        loader_name,
                    });
                }
                Err(e) => {
                    self.states
                        .insert(plugin_id.clone(), PluginState::Error(e.to_string()));
                    tracing::error!(plugin = %plugin_id, error = %e, "Plugin failed to enable");

                    if let Some(ctx_impl) = ctx.as_any().downcast_ref::<PluginContextImpl>() {
                        ctx_impl.cleanup();
                    }
                    errors.push(e);
                }
            }
        }

        errors
    }

    /// Disables all plugins in reverse order, then unloads via loaders.
    pub async fn shutdown(&mut self) {
        for loaded in self.plugins.iter().rev() {
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

            // Cleanup is always executed, even if on_disable errored
            if let Some(ctx_impl) = loaded.context.as_any().downcast_ref::<PluginContextImpl>() {
                ctx_impl.cleanup();
            }
        }

        for loaded in self.plugins.iter().rev() {
            let loader = self.loaders.iter().find(|l| l.name() == loaded.loader_name);

            if let Some(loader) = loader
                && let Err(e) = loader.unload(&loaded.metadata.id).await
            {
                tracing::error!(
                    plugin = %loaded.metadata.id,
                    error = %e,
                    "Loader unload failed"
                );
            }
        }

        let loaded = std::mem::take(&mut self.loaded_loaders);
        for name in loaded.iter().rev() {
            if let Some(loader) = self.loaders.iter().find(|l| l.name() == name)
                && let Err(e) = loader.on_shutdown().await
            {
                tracing::error!(loader = %name, error = %e, "Loader on_shutdown() failed");
            }
        }
    }

    /// Collects all limbo handlers from enabled plugins.
    /// Call exactly once after `load_and_enable_all()`.
    pub fn collect_limbo_handlers(&self) -> Vec<Box<dyn infrarust_api::limbo::LimboHandler>> {
        let mut all = Vec::new();
        for loaded in &self.plugins {
            if let Some(ctx_impl) = loaded.context.as_any().downcast_ref::<PluginContextImpl>() {
                all.extend(ctx_impl.take_limbo_handlers());
            }
        }
        all
    }

    pub fn collect_config_providers(
        &self,
    ) -> Vec<(
        String,
        Box<dyn infrarust_api::provider::PluginConfigProvider>,
    )> {
        let mut all = Vec::new();
        for loaded in &self.plugins {
            if let Some(ctx_impl) = loaded.context.as_any().downcast_ref::<PluginContextImpl>() {
                let providers = ctx_impl.take_config_providers();
                for provider in providers {
                    all.push((loaded.metadata.id.clone(), provider));
                }
            }
        }
        all
    }

    pub fn store_provider_cleanup(
        &self,
        results: Vec<(String, crate::provider::plugin_adapter::ActivatedProvider)>,
    ) {
        for (plugin_id, activated) in results {
            for loaded in &self.plugins {
                if loaded.metadata.id == plugin_id {
                    if let Some(ctx_impl) =
                        loaded.context.as_any().downcast_ref::<PluginContextImpl>()
                    {
                        ctx_impl.register_active_provider_ids(activated.config_ids);
                        ctx_impl.register_provider_token(activated.watch_token);
                    }
                    break;
                }
            }
        }
    }

    pub fn is_plugin_loaded(&self, id: &str) -> bool {
        matches!(self.states.get(id), Some(PluginState::Enabled))
    }

    pub fn plugin_state(&self, id: &str) -> Option<&PluginState> {
        self.states.get(id)
    }

    pub fn list_plugins(&self) -> Vec<&PluginMetadata> {
        self.plugins.iter().map(|p| &p.metadata).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use infrarust_api::error::PluginError;
    use infrarust_api::event::BoxFuture;
    use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};

    use crate::plugin::context_factory::PluginContextFactory;
    use crate::plugin::static_loader::StaticPluginLoader;

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
        capabilities: infrarust_api::permissions::CapabilitySet,
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
        fn data_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::from("plugins").join(&self.plugin_id)
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
        fn proxy_info(&self) -> &infrarust_api::services::proxy_info::ProxyInfo {
            unimplemented!("mock")
        }
        fn capabilities(&self) -> &infrarust_api::permissions::CapabilitySet {
            &self.capabilities
        }
    }

    struct MockPluginContextFactory;

    impl PluginContextFactory for MockPluginContextFactory {
        fn create_context(&self, plugin_id: &str) -> Arc<dyn PluginContext> {
            Arc::new(MockPluginContext {
                plugin_id: plugin_id.to_string(),
                capabilities: infrarust_api::permissions::CapabilitySet::native_trusted(),
            })
        }
    }

    #[tokio::test]
    async fn test_plugin_manager_discovers_from_multiple_loaders() {
        let loader_a = StaticPluginLoader::new();
        loader_a.register(PluginMetadata::new("plugin_a", "A", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "plugin_a".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });

        let loader_b = StaticPluginLoader::new();
        loader_b.register(PluginMetadata::new("plugin_b", "B", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "plugin_b".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });

        let mut manager = PluginManager::new(vec![Box::new(loader_a), Box::new(loader_b)]);

        let discovered = manager.discover_all(Path::new("plugins")).await.unwrap();
        assert_eq!(discovered.len(), 2);
    }

    #[tokio::test]
    async fn test_plugin_manager_detects_duplicate_ids_across_loaders() {
        let loader_a = StaticPluginLoader::new();
        loader_a.register(PluginMetadata::new("conflict", "A", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "conflict".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });

        let loader_b = StaticPluginLoader::new();
        loader_b.register(PluginMetadata::new("conflict", "B", "1.0.0"), || {
            Box::new(TestPlugin {
                id: "conflict".into(),
                enabled: Arc::new(AtomicBool::new(false)),
            })
        });

        let mut manager = PluginManager::new(vec![Box::new(loader_a), Box::new(loader_b)]);

        let result = manager.discover_all(Path::new("plugins")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_manager_full_lifecycle() {
        let loader = StaticPluginLoader::new();
        let enabled = Arc::new(AtomicBool::new(false));
        let enabled_clone = enabled.clone();

        loader.register(
            PluginMetadata::new("lifecycle_test", "Lifecycle Test", "1.0.0"),
            move || {
                Box::new(TestPlugin {
                    id: "lifecycle_test".into(),
                    enabled: enabled_clone.clone(),
                })
            },
        );

        let mut manager = PluginManager::new(vec![Box::new(loader)]);
        let factory = MockPluginContextFactory;

        manager.discover_all(Path::new("plugins")).await.unwrap();
        let errors = manager.load_and_enable_all(&factory).await;
        assert!(errors.is_empty());
        assert!(enabled.load(Ordering::Relaxed));

        manager.shutdown().await;
        assert!(!enabled.load(Ordering::Relaxed));
    }
    use std::sync::Mutex;

    use infrarust_api::loader::LoaderError;

    struct LifecycleLoader {
        name: &'static str,
        log: Arc<Mutex<Vec<String>>>,
        metadatas: Vec<PluginMetadata>,
        fail_on_load: bool,
    }

    impl PluginLoader for LifecycleLoader {
        fn name(&self) -> &str {
            self.name
        }

        fn discover<'a>(
            &'a self,
            _dir: &'a Path,
        ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>> {
            let metas = self.metadatas.clone();
            Box::pin(async move { Ok(metas) })
        }

        fn on_load<'a>(
            &'a self,
            _factory: &'a dyn PluginContextFactory,
        ) -> BoxFuture<'a, Result<(), LoaderError>> {
            let log = Arc::clone(&self.log);
            let label = format!("on_load:{}", self.name);
            let fail = self.fail_on_load;
            let name = self.name;
            Box::pin(async move {
                log.lock().expect("lock poisoned").push(label);
                if fail {
                    Err(LoaderError::LoadFailed {
                        plugin_id: name.to_string(),
                        reason: "boom".to_string(),
                        source: None,
                    })
                } else {
                    Ok(())
                }
            })
        }

        fn load<'a>(
            &'a self,
            plugin_id: &'a str,
            _factory: &'a dyn PluginContextFactory,
        ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
            let log = Arc::clone(&self.log);
            let id = plugin_id.to_string();
            Box::pin(async move {
                log.lock().expect("lock poisoned").push(format!("load:{id}"));
                Ok(Box::new(TestPlugin {
                    id,
                    enabled: Arc::new(AtomicBool::new(false)),
                }) as Box<dyn Plugin>)
            })
        }

        fn unload<'a>(&'a self, plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
            let log = Arc::clone(&self.log);
            let id = plugin_id.to_string();
            Box::pin(async move {
                log.lock()
                    .expect("lock poisoned")
                    .push(format!("unload:{id}"));
                Ok(())
            })
        }

        fn on_shutdown<'a>(&'a self) -> BoxFuture<'a, Result<(), LoaderError>> {
            let log = Arc::clone(&self.log);
            let label = format!("on_shutdown:{}", self.name);
            Box::pin(async move {
                log.lock().expect("lock poisoned").push(label);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn on_load_runs_before_loads_and_on_shutdown_after_unloads() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let loader = LifecycleLoader {
            name: "lifecycle",
            log: Arc::clone(&log),
            metadatas: vec![PluginMetadata::new("p1", "P1", "1.0.0")],
            fail_on_load: false,
        };
        let mut mgr = PluginManager::new(vec![Box::new(loader)]);
        mgr.discover_all(Path::new("plugins")).await.unwrap();
        let errors = mgr.load_and_enable_all(&MockPluginContextFactory).await;
        assert!(errors.is_empty());
        mgr.shutdown().await;

        let seq = log.lock().unwrap().clone();
        assert_eq!(
            seq,
            vec![
                "on_load:lifecycle",
                "load:p1",
                "unload:p1",
                "on_shutdown:lifecycle",
            ]
        );
    }

    #[tokio::test]
    async fn loader_with_zero_plugins_still_initialises_and_tears_down() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let loader = LifecycleLoader {
            name: "host",
            log: Arc::clone(&log),
            metadatas: vec![],
            fail_on_load: false,
        };
        let mut mgr = PluginManager::new(vec![Box::new(loader)]);
        mgr.discover_all(Path::new("plugins")).await.unwrap();
        let errors = mgr.load_and_enable_all(&MockPluginContextFactory).await;
        assert!(errors.is_empty());
        mgr.shutdown().await;

        // A host that booted with zero plugins must still be torn down.
        let seq = log.lock().unwrap().clone();
        assert_eq!(seq, vec!["on_load:host", "on_shutdown:host"]);
    }

    #[tokio::test]
    async fn failing_on_load_skips_its_plugins_and_skips_its_shutdown() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let bad = LifecycleLoader {
            name: "bad",
            log: Arc::clone(&log),
            metadatas: vec![PluginMetadata::new("b1", "B1", "1.0.0")],
            fail_on_load: true,
        };
        let good = LifecycleLoader {
            name: "good",
            log: Arc::clone(&log),
            metadatas: vec![PluginMetadata::new("g1", "G1", "1.0.0")],
            fail_on_load: false,
        };
        let mut mgr = PluginManager::new(vec![Box::new(bad), Box::new(good)]);
        mgr.discover_all(Path::new("plugins")).await.unwrap();
        let errors = mgr.load_and_enable_all(&MockPluginContextFactory).await;

        // The failed on_load surfaces exactly one error...
        assert_eq!(errors.len(), 1);
        // ...its plugin is marked Error and never loaded...
        assert!(matches!(mgr.plugin_state("b1"), Some(PluginState::Error(_))));
        // ...while the healthy loader's plugin still enables.
        assert!(mgr.is_plugin_loaded("g1"));

        mgr.shutdown().await;
        let seq = log.lock().unwrap().clone();
        // bad: on_load fired, but no load and no on_shutdown (it never booted).
        assert!(seq.contains(&"on_load:bad".to_string()));
        assert!(!seq.iter().any(|s| s == "load:b1"));
        assert!(!seq.iter().any(|s| s == "on_shutdown:bad"));
        // good: full lifecycle ran.
        assert!(seq.contains(&"load:g1".to_string()));
        assert!(seq.contains(&"on_shutdown:good".to_string()));
    }

    #[tokio::test]
    async fn second_shutdown_does_not_repeat_on_shutdown() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let loader = LifecycleLoader {
            name: "once",
            log: Arc::clone(&log),
            metadatas: vec![PluginMetadata::new("p1", "P1", "1.0.0")],
            fail_on_load: false,
        };
        let mut mgr = PluginManager::new(vec![Box::new(loader)]);
        mgr.discover_all(Path::new("plugins")).await.unwrap();
        mgr.load_and_enable_all(&MockPluginContextFactory).await;
        mgr.shutdown().await;
        mgr.shutdown().await;

        let count = log
            .lock()
            .unwrap()
            .iter()
            .filter(|s| *s == "on_shutdown:once")
            .count();
        assert_eq!(count, 1);
    }
}
