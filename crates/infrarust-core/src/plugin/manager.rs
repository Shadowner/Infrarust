//! [`PluginManager`] — orchestrates plugin lifecycle.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use infrarust_api::error::PluginError;
use infrarust_api::event::Event;
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::{
    ban_service::BanService, config_service::ConfigService, load_balancer::LoadBalancerService,
    player_registry::PlayerRegistry, plugin_registry::PluginRegistry, proxy_info::ProxyInfo,
    server_manager::ServerManager,
};
use tokio_util::sync::CancellationToken;

use crate::event_bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_registry::TransportFilterRegistryImpl;
use crate::services::scheduler::SchedulerImpl;

use super::PluginRegistryImpl;
use super::PluginState;
use super::context::PluginContextImpl;
use super::context_factory::{PluginContextFactory, PluginContextFactoryImpl};
use super::dependency::resolve_load_order;
use super::loader::PluginLoader;

/// Services required to construct per-plugin contexts.
pub struct PluginServices {
    pub event_bus: Arc<EventBusImpl>,
    pub player_registry: Arc<dyn PlayerRegistry>,
    pub server_manager: Arc<dyn ServerManager>,
    pub ban_service: Arc<dyn BanService>,
    pub command_manager: Arc<crate::services::command_manager::CommandManagerImpl>,
    pub scheduler: Arc<SchedulerImpl>,
    pub config_service: Arc<dyn ConfigService>,
    pub load_balancer_service: Arc<dyn LoadBalancerService>,
    pub plugin_registry: Arc<dyn PluginRegistry>,
    pub codec_filter_registry: Arc<CodecFilterRegistryImpl>,
    pub transport_filter_registry: Arc<TransportFilterRegistryImpl>,
    pub domain_router: Arc<crate::routing::DomainRouter>,
    pub proxy_shutdown: CancellationToken,
    pub proxy_info: ProxyInfo,
    pub plugins_dir: PathBuf,
}

type LoaderIndex = usize;

pub struct PluginManager {
    loaders: Vec<Box<dyn PluginLoader>>,
    plugins: Vec<LoadedPlugin>,
    states: HashMap<String, PluginState>,
    load_order: Vec<String>,
    loader_of: HashMap<String, LoaderIndex>,
    loaded_loaders: Vec<LoaderIndex>,
    disabled: HashSet<String>,
    event_bus: Option<Arc<EventBusImpl>>,
    registry: Option<Arc<PluginRegistryImpl>>,
    context_factory: Option<Arc<PluginContextFactoryImpl>>,
}

struct LoadedPlugin {
    plugin: Box<dyn Plugin>,
    context: Arc<PluginContextImpl>,
    metadata: PluginMetadata,
    loader: LoaderIndex,
}

impl PluginManager {
    pub fn new(loaders: Vec<Box<dyn PluginLoader>>) -> Self {
        Self {
            loaders,
            plugins: Vec::new(),
            states: HashMap::new(),
            load_order: Vec::new(),
            loader_of: HashMap::new(),
            loaded_loaders: Vec::new(),
            disabled: HashSet::new(),
            event_bus: None,
            registry: None,
            context_factory: None,
        }
    }

    pub fn set_event_bus(&mut self, event_bus: Arc<EventBusImpl>) {
        self.event_bus = Some(event_bus);
    }

    pub fn set_plugin_registry(&mut self, registry: Arc<PluginRegistryImpl>) {
        self.registry = Some(registry);
    }

    async fn announce<E: Event>(&self, event: E) {
        if let Some(bus) = &self.event_bus {
            bus.post(event);
            bus.flush().await;
        }
    }

    pub fn set_disabled_plugins(&mut self, ids: HashSet<String>) {
        self.disabled = ids;
    }

    /// Discovers all plugins via loaders, detects duplicate IDs,
    /// and resolves load order via topological sort.
    pub async fn discover_all(
        &mut self,
        plugin_dir: &Path,
    ) -> Result<Vec<PluginMetadata>, PluginError> {
        let mut all_metadata: Vec<PluginMetadata> = Vec::new();
        let mut loader_of: HashMap<String, LoaderIndex> = HashMap::new();

        for (at, loader) in self.loaders.iter().enumerate() {
            let discovered = loader.discover(plugin_dir).await.map_err(|e| {
                PluginError::InitFailed(format!("Loader '{}' discovery failed: {e}", loader.name()))
            })?;

            for metadata in discovered {
                if let Some(existing) = loader_of.get(&metadata.id) {
                    return Err(PluginError::InitFailed(format!(
                        "Duplicate plugin id '{}': found in loader '{}' and '{}'",
                        metadata.id,
                        self.loaders[*existing].name(),
                        loader.name()
                    )));
                }
                loader_of.insert(metadata.id.clone(), at);
                all_metadata.push(metadata);
            }
        }

        let load_order = resolve_load_order(&all_metadata)?;

        self.load_order = load_order;
        self.loader_of = loader_of;

        Ok(all_metadata)
    }

    pub async fn load_and_enable_all(
        &mut self,
        context_factory: Arc<PluginContextFactoryImpl>,
    ) -> Vec<PluginError> {
        let mut errors = Vec::new();
        let load_order = self.load_order.clone();
        let factory: &dyn PluginContextFactory = context_factory.as_ref();

        let mut failed_loaders: HashSet<LoaderIndex> = HashSet::new();
        let mut ok_loaders: Vec<LoaderIndex> = Vec::new();
        for (at, loader) in self.loaders.iter().enumerate() {
            match loader.on_load(factory).await {
                Ok(()) => ok_loaders.push(at),
                Err(e) => {
                    let name = loader.name();
                    tracing::error!(loader = %name, error = %e, "Loader on_load() failed");
                    errors.push(PluginError::InitFailed(format!(
                        "Loader '{name}' on_load failed: {e}"
                    )));
                    failed_loaders.insert(at);
                }
            }
        }
        self.loaded_loaders = ok_loaders;

        for plugin_id in &load_order {
            if self.disabled.contains(plugin_id) {
                tracing::info!(plugin = %plugin_id, "Plugin disabled via config, skipping");
                self.states.insert(plugin_id.clone(), PluginState::Disabled);
                continue;
            }
            let Some(&at) = self.loader_of.get(plugin_id) else {
                errors.push(PluginError::InitFailed(format!(
                    "No loader mapping for plugin '{plugin_id}'"
                )));
                continue;
            };
            let loader = &self.loaders[at];
            let loader_name = loader.name();

            if failed_loaders.contains(&at) {
                self.states.insert(
                    plugin_id.clone(),
                    PluginState::Error(format!("loader '{loader_name}' on_load failed")),
                );
                continue;
            }

            let plugin = match loader.load(plugin_id, factory).await {
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
            let ctx = context_factory.context(plugin_id);

            self.states.insert(plugin_id.clone(), PluginState::Loading);
            match plugin.on_enable(ctx.as_ref()).await {
                Ok(()) => {
                    self.states.insert(plugin_id.clone(), PluginState::Enabled);
                    tracing::info!(plugin = %plugin_id, "Plugin enabled");
                    if let Some(registry) = &self.registry {
                        registry.insert_enabled(&metadata);
                    }
                    self.announce(PluginEnabledEvent::new(
                        plugin_id.clone(),
                        metadata.version.clone(),
                    ))
                    .await;

                    self.plugins.push(LoadedPlugin {
                        plugin,
                        context: ctx,
                        metadata,
                        loader: at,
                    });
                }
                Err(e) => {
                    self.states
                        .insert(plugin_id.clone(), PluginState::Error(e.to_string()));
                    tracing::error!(plugin = %plugin_id, error = %e, "Plugin failed to enable");
                    ctx.cleanup();
                    context_factory.forget_context(plugin_id);
                    errors.push(e);
                }
            }
        }

        self.context_factory = Some(context_factory);
        errors
    }

    /// Disables all plugins in reverse order, then unloads via loaders.
    pub async fn shutdown(&mut self) {
        let plugins = std::mem::take(&mut self.plugins);

        for loaded in plugins.iter().rev() {
            self.disable_loaded(loaded).await;
        }

        for loaded in plugins.iter().rev() {
            self.unload(loaded).await;
        }

        let loaded = std::mem::take(&mut self.loaded_loaders);
        for at in loaded.into_iter().rev() {
            let loader = &self.loaders[at];
            if let Err(e) = loader.on_shutdown().await {
                tracing::error!(loader = %loader.name(), error = %e, "Loader on_shutdown() failed");
            }
        }
    }

    pub async fn disable_plugin(&mut self, id: &str) -> Result<(), PluginError> {
        let Some(at) = self.plugins.iter().position(|p| p.metadata.id == id) else {
            return Err(PluginError::from(format!("plugin `{id}` is not enabled")));
        };
        if let Some(dependent) = self.plugins.iter().find(|p| {
            p.metadata.id != id
                && matches!(self.states.get(&p.metadata.id), Some(PluginState::Enabled))
                && p.metadata
                    .dependencies
                    .iter()
                    .any(|dep| dep.id == id && !dep.optional)
        }) {
            return Err(PluginError::from(format!(
                "plugin `{id}` is required by `{}`",
                dependent.metadata.id
            )));
        }
        let loaded = self.plugins.remove(at);
        self.disable_loaded(&loaded).await;
        self.unload(&loaded).await;
        Ok(())
    }

    async fn disable_loaded(&mut self, loaded: &LoadedPlugin) {
        let id = &loaded.metadata.id;
        if !matches!(self.states.get(id), Some(PluginState::Enabled)) {
            return;
        }

        tracing::info!(plugin = %id, "Disabling plugin");
        self.states.insert(id.clone(), PluginState::Disabled);
        if let Some(registry) = &self.registry {
            registry.remove(id);
        }

        if let Err(e) = loaded.plugin.on_disable().await {
            tracing::error!(plugin = %id, error = %e, "Plugin on_disable() failed");
        }

        loaded.context.cleanup();
        self.forget_context(id);
        self.announce(PluginDisabledEvent::new(id.clone())).await;
    }

    async fn unload(&self, loaded: &LoadedPlugin) {
        let id = &loaded.metadata.id;
        if let Err(e) = self.loaders[loaded.loader].unload(id).await {
            tracing::error!(plugin = %id, error = %e, "Loader unload failed");
        }
        self.forget_context(id);
    }

    fn forget_context(&self, id: &str) {
        if let Some(factory) = &self.context_factory {
            factory.forget_context(id);
        }
    }

    pub fn collect_config_providers(
        &self,
    ) -> Vec<(
        String,
        Box<dyn infrarust_api::provider::PluginConfigProvider>,
    )> {
        let mut all = Vec::new();
        for loaded in &self.plugins {
            for provider in loaded.context.take_config_providers() {
                all.push((loaded.metadata.id.clone(), provider));
            }
        }
        all
    }

    pub fn store_provider_cleanup(
        &self,
        results: Vec<(String, crate::provider::plugin_adapter::ActivatedProvider)>,
    ) {
        let contexts: HashMap<&str, &Arc<PluginContextImpl>> = self
            .plugins
            .iter()
            .map(|loaded| (loaded.metadata.id.as_str(), &loaded.context))
            .collect();
        for (plugin_id, activated) in results {
            if let Some(ctx) = contexts.get(plugin_id.as_str()) {
                ctx.register_active_provider_ids(activated.config_ids);
                ctx.register_provider_token(activated.watch_token);
            }
        }
    }

    pub fn plugin_context(&self, id: &str) -> Option<Arc<dyn PluginContext>> {
        self.plugins
            .iter()
            .find(|loaded| loaded.metadata.id == id)
            .map(|loaded| Arc::clone(&loaded.context) as Arc<dyn PluginContext>)
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::event::BoxFuture;

    use crate::plugin::static_loader::StaticPluginLoader;
    use crate::test_support::TestPlugin;
    use crate::util::sync::lock;

    use super::*;

    fn factory() -> Arc<PluginContextFactoryImpl> {
        Arc::new(PluginContextFactoryImpl::new(
            PluginServices::for_tests(),
            HashMap::new(),
        ))
    }

    #[tokio::test]
    async fn test_plugin_manager_discovers_from_multiple_loaders() {
        let loader_a = StaticPluginLoader::new();
        loader_a.register(PluginMetadata::new("plugin_a", "A", "1.0.0"), || {
            Box::new(TestPlugin::new("plugin_a"))
        });

        let loader_b = StaticPluginLoader::new();
        loader_b.register(PluginMetadata::new("plugin_b", "B", "1.0.0"), || {
            Box::new(TestPlugin::new("plugin_b"))
        });

        let mut manager = PluginManager::new(vec![Box::new(loader_a), Box::new(loader_b)]);

        let discovered = manager.discover_all(Path::new("plugins")).await.unwrap();
        assert_eq!(discovered.len(), 2);
    }

    #[tokio::test]
    async fn test_plugin_manager_detects_duplicate_ids_across_loaders() {
        let loader_a = StaticPluginLoader::new();
        loader_a.register(PluginMetadata::new("conflict", "A", "1.0.0"), || {
            Box::new(TestPlugin::new("conflict"))
        });

        let loader_b = StaticPluginLoader::new();
        loader_b.register(PluginMetadata::new("conflict", "B", "1.0.0"), || {
            Box::new(TestPlugin::new("conflict"))
        });

        let mut manager = PluginManager::new(vec![Box::new(loader_a), Box::new(loader_b)]);

        let result = manager.discover_all(Path::new("plugins")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_manager_full_lifecycle() {
        let loader = StaticPluginLoader::new();
        let plugin = TestPlugin::new("lifecycle_test");
        plugin.register(&loader);

        let mut manager = PluginManager::new(vec![Box::new(loader)]);

        manager.discover_all(Path::new("plugins")).await.unwrap();
        let errors = manager.load_and_enable_all(factory()).await;
        assert!(errors.is_empty());
        assert!(plugin.is_enabled());

        manager.shutdown().await;
        assert!(!plugin.is_enabled());
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
                lock(&log).push(label);
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
                lock(&log).push(format!("load:{id}"));
                Ok(Box::new(TestPlugin::new(&id)) as Box<dyn Plugin>)
            })
        }

        fn unload<'a>(&'a self, plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
            let log = Arc::clone(&self.log);
            let id = plugin_id.to_string();
            Box::pin(async move {
                lock(&log).push(format!("unload:{id}"));
                Ok(())
            })
        }

        fn on_shutdown<'a>(&'a self) -> BoxFuture<'a, Result<(), LoaderError>> {
            let log = Arc::clone(&self.log);
            let label = format!("on_shutdown:{}", self.name);
            Box::pin(async move {
                lock(&log).push(label);
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
        let errors = mgr.load_and_enable_all(factory()).await;
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
        let errors = mgr.load_and_enable_all(factory()).await;
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
        let errors = mgr.load_and_enable_all(factory()).await;

        // The failed on_load surfaces exactly one error...
        assert_eq!(errors.len(), 1);
        // ...its plugin is marked Error and never loaded...
        assert!(matches!(
            mgr.plugin_state("b1"),
            Some(PluginState::Error(_))
        ));
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
        mgr.load_and_enable_all(factory()).await;
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
