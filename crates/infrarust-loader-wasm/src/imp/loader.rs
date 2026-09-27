use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, Weak};

use infrarust_api::event::BoxFuture;
use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::permissions::Capability;
use infrarust_api::plugin::{Plugin, PluginMetadata};
use wasmtime::Engine;
use wasmtime::component::Component;

use crate::actor::PluginActor;
use crate::cache::AotCache;
use crate::config::WasmLoaderConfig;
use crate::consts::CACHE_SUBDIR;
use crate::contract::check as check_contract;
use crate::epoch::EpochTicker;
use crate::error::WasmLoaderError;
use crate::gates::check_imports;
use crate::instance::InstanceFactory;
use crate::linker::build_linker;
use crate::metadata::extract_metadata;
use crate::plugin::WasmPlugin;
use crate::registrations::Registrations;
use crate::store_state::PluginSetup;
use crate::sync::{lock, read, write};

pub struct WasmPluginLoader {
    engine: Engine,
    config: WasmLoaderConfig,
    discovered: RwLock<HashMap<String, DiscoveredWasm>>,
    actors: Mutex<HashMap<String, Weak<PluginActor>>>,
    _ticker: EpochTicker,
}

#[derive(Clone)]
struct DiscoveredWasm {
    metadata: PluginMetadata,
    component: Component,
    path: PathBuf,
}

impl WasmPluginLoader {
    pub fn new(engine: Engine, config: WasmLoaderConfig) -> std::io::Result<Self> {
        let ticker = EpochTicker::spawn(engine.clone(), config.epoch_tick())?;
        Ok(Self {
            engine,
            config,
            discovered: RwLock::new(HashMap::new()),
            actors: Mutex::new(HashMap::new()),
            _ticker: ticker,
        })
    }

    async fn probe(
        &self,
        cache: &AotCache,
        path: &Path,
    ) -> Result<DiscoveredWasm, WasmLoaderError> {
        let component = {
            let engine = self.engine.clone();
            let cache = cache.clone();
            let owned = path.to_path_buf();
            tokio::task::spawn_blocking(move || cache.compile_or_load(&engine, &owned))
                .await
                .map_err(|join_err| WasmLoaderError::Precompile {
                    path: path.to_path_buf(),
                    reason: format!("compile task failed: {join_err}"),
                })??
        };
        check_contract(&self.engine, &component, path)?;
        let metadata = extract_metadata(
            &self.engine,
            &component,
            path,
            &self.config.default_sandbox(),
        )
        .await?;
        Ok(DiscoveredWasm {
            metadata,
            component,
            path: path.to_path_buf(),
        })
    }
}

fn without_duplicate_ids(
    probed: Vec<DiscoveredWasm>,
) -> (Vec<PluginMetadata>, HashMap<String, DiscoveredWasm>) {
    let mut ids = Vec::new();
    let mut files_of: HashMap<String, Vec<String>> = HashMap::new();
    for entry in &probed {
        let files = files_of.entry(entry.metadata.id.clone()).or_default();
        if files.is_empty() {
            ids.push(entry.metadata.id.clone());
        }
        files.push(entry.path.display().to_string());
    }
    for id in &ids {
        if let Some(files) = files_of.get(id).filter(|files| files.len() > 1) {
            tracing::error!(
                plugin = %id,
                files = %files.join(", "),
                "WASM plugins refused: several files declare the same plugin id, keep only one of them"
            );
        }
    }

    let mut metadatas = Vec::new();
    let mut discovered = HashMap::new();
    for entry in probed {
        let id = entry.metadata.id.clone();
        if files_of.get(&id).is_some_and(|files| files.len() > 1) {
            continue;
        }
        tracing::debug!(plugin = %id, path = %entry.path.display(), "wasm plugin discovered");
        metadatas.push(entry.metadata.clone());
        discovered.insert(id, entry);
    }
    (metadatas, discovered)
}

impl PluginLoader for WasmPluginLoader {
    fn name(&self) -> &str {
        "wasm"
    }

    fn discover<'a>(
        &'a self,
        plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>> {
        Box::pin(async move {
            if !plugin_dir.exists() {
                return Ok(Vec::new());
            }
            let cache = AotCache::new(plugin_dir.join(CACHE_SUBDIR));
            let wasm_files = scan_wasm_files(plugin_dir)?;

            let mut probed = Vec::new();
            for path in wasm_files {
                match self.probe(&cache, &path).await {
                    Ok(entry) => probed.push(entry),
                    Err(error) => {
                        tracing::error!(
                            path = %path.display(),
                            error = %error,
                            "WASM plugin refused"
                        );
                    }
                }
            }

            let (metadatas, discovered) = without_duplicate_ids(probed);
            *write(&self.discovered) = discovered;
            Ok(metadatas)
        })
    }

    fn plugin_source(&self, plugin_id: &str) -> Option<PathBuf> {
        read(&self.discovered)
            .get(plugin_id)
            .map(|entry| entry.path.clone())
    }

    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
        Box::pin(async move {
            let entry = read(&self.discovered)
                .get(plugin_id)
                .cloned()
                .ok_or_else(|| LoaderError::PluginNotFound {
                    plugin_id: plugin_id.to_owned(),
                })?;

            let ctx = context_factory.create_context(plugin_id);
            let capabilities = ctx.capabilities().clone();
            let data_dir = ctx.data_dir();
            let sandbox = self.config.sandbox_for(plugin_id);
            let network = crate::network::policy_for(
                plugin_id,
                self.config.network_for(plugin_id),
                &capabilities,
                sandbox.host_call_timeout,
            );
            let mounts = crate::mounts::resolve_mounts(
                plugin_id,
                self.config.mounts_for(plugin_id),
                &capabilities,
            )
            .map_err(|e| e.into_loader_error(plugin_id))?;

            check_imports(
                &self.engine,
                &entry.component,
                plugin_id,
                &capabilities,
                self.config.strict_capabilities(plugin_id),
            )
            .map_err(|e| e.into_loader_error(plugin_id))?;
            let linker = build_linker(&self.engine, plugin_id)
                .map_err(|e| e.into_loader_error(plugin_id))?;

            let codec = if capabilities.has(Capability::CodecFilter) {
                let instantiator = crate::codec::CodecInstantiator::new(
                    self.engine.clone(),
                    &entry.component,
                    plugin_id.to_owned(),
                    &sandbox,
                )
                .map_err(|e| e.into_loader_error(plugin_id))?;
                Some(Arc::new(instantiator))
            } else {
                None
            };

            let shutdown = ctx.proxy_shutdown();
            let shutting_down = Box::new(move || shutdown.is_cancelled());
            let setup = PluginSetup {
                plugin_id: plugin_id.to_owned(),
                ctx,
                capabilities,
                data_dir,
                codec,
                sandbox,
                registrations: Arc::new(Registrations::default()),
                network,
                mounts: mounts.into(),
            };
            let factory =
                InstanceFactory::new(self.engine.clone(), &entry.component, &linker, setup)
                    .map_err(|e| e.into_loader_error(plugin_id))?;
            let actor = PluginActor::start(factory)
                .await
                .map_err(|e| e.into_loader_error(plugin_id))?;
            lock(&self.actors).insert(plugin_id.to_owned(), Arc::downgrade(&actor));
            Ok(Box::new(WasmPlugin::new(entry.metadata, actor, shutting_down)) as Box<dyn Plugin>)
        })
    }

    fn unload<'a>(&'a self, plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async move {
            let actor = lock(&self.actors)
                .remove(plugin_id)
                .and_then(|actor| actor.upgrade());
            if let Some(actor) = actor {
                actor.shutdown().await;
            }
            tracing::debug!(plugin = %plugin_id, "wasm plugin unloaded");
            Ok(())
        })
    }
}

fn scan_wasm_files(dir: &Path) -> Result<Vec<PathBuf>, LoaderError> {
    let mut entries = std::fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()
        })
        .map_err(|source| LoaderError::DirectoryNotAccessible {
            path: dir.to_path_buf(),
            source,
        })?;
    entries.sort();

    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(metadata) if metadata.is_dir() => continue,
            Ok(_) => {
                tracing::error!(
                    path = %path.display(),
                    error = "not a regular file",
                    "WASM plugin refused"
                );
                continue;
            }
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    error = %error,
                    "WASM plugin refused"
                );
                continue;
            }
        }
        let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(identity) {
            files.push(path);
        } else {
            tracing::debug!(
                path = %path.display(),
                "plugin file already found through another link, skipped"
            );
        }
    }
    Ok(files)
}
