use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use infrarust_api::event::BoxFuture;
use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::permissions::Capability;
use infrarust_api::plugin::{Plugin, PluginMetadata};
use wasmtime::component::Component;
use wasmtime::{Engine, Store};

use crate::bindings::Plugin as PluginBindings;
use crate::cache::AotCache;
use crate::consts::CACHE_SUBDIR;
use crate::epoch::EpochTicker;
use crate::error::WasmLoaderError;
use crate::linker::build_linker;
use crate::metadata::extract_metadata;
use crate::plugin::WasmPlugin;
use crate::store_state::{PluginStoreState, build_load_state, install_epoch_control};

pub struct WasmPluginLoader {
    engine: Engine,
    discovered: RwLock<HashMap<String, DiscoveredWasm>>,
    // RAII guard: never read, but its `Drop` stops the epoch-ticker thread.
    #[allow(dead_code)]
    ticker: EpochTicker,
}

#[derive(Clone)]
struct DiscoveredWasm {
    metadata: PluginMetadata,
    component: Component,
}

impl WasmPluginLoader {
    pub fn new(engine: Engine) -> Self {
        let ticker = EpochTicker::spawn(engine.clone());
        Self {
            engine,
            discovered: RwLock::new(HashMap::new()),
            ticker,
        }
    }
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

            let mut metadatas = Vec::new();
            let mut discovered = HashMap::new();
            for path in wasm_files {
                let label = path_label(&path);

                let component = {
                    let engine = self.engine.clone();
                    let cache = cache.clone();
                    let path = path.clone();
                    tokio::task::spawn_blocking(move || cache.compile_or_load(&engine, &path))
                        .await
                        .map_err(|join_err| LoaderError::LoadFailed {
                            plugin_id: label.clone(),
                            reason: format!("compile task failed: {join_err}"),
                            source: None,
                        })?
                        .map_err(|e| e.into_loader_error(&label))?
                };

                let metadata = extract_metadata(&self.engine, &component, &path)
                    .await
                    .map_err(|e| e.into_loader_error(&label))?;

                metadatas.push(metadata.clone());
                discovered.insert(
                    metadata.id.clone(),
                    DiscoveredWasm {
                        metadata,
                        component,
                    },
                );
            }

            *self.discovered.write().expect("discovered lock poisoned") = discovered;
            Ok(metadatas)
        })
    }

    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
        Box::pin(async move {
            let entry = self
                .discovered
                .read()
                .expect("discovered lock poisoned")
                .get(plugin_id)
                .cloned()
                .ok_or_else(|| LoaderError::PluginNotFound {
                    plugin_id: plugin_id.to_owned(),
                })?;

            let ctx = context_factory.create_context(plugin_id);
            let capabilities = ctx.capabilities().clone();
            let data_dir = ctx.data_dir();

            let linker = build_linker(&self.engine, plugin_id, &capabilities)
                .map_err(|e| e.into_loader_error(plugin_id))?;

            let codec = if capabilities.has(Capability::CodecFilter) {
                let instantiator = crate::codec::CodecInstantiator::new(
                    self.engine.clone(),
                    &entry.component,
                    plugin_id.to_owned(),
                )
                .map_err(|e| e.into_loader_error(plugin_id))?;
                Some(Arc::new(instantiator))
            } else {
                None
            };

            let state = build_load_state(plugin_id.to_owned(), ctx, capabilities, &data_dir, codec)
                .map_err(|e| e.into_loader_error(plugin_id))?;
            let mut store = Store::new(&self.engine, state);
            install_epoch_control(&mut store);
            store.limiter(|s: &mut PluginStoreState| {
                s.limits_mut() as &mut dyn wasmtime::ResourceLimiter
            });

            let bindings = PluginBindings::instantiate_async(&mut store, &entry.component, &linker)
                .await
                .map_err(|e| map_instantiate_error(plugin_id, &e).into_loader_error(plugin_id))?;

            Ok(Box::new(WasmPlugin::new(entry.metadata, store, bindings)) as Box<dyn Plugin>)
        })
    }

    fn unload<'a>(&'a self, plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async move {
            tracing::debug!(plugin = %plugin_id, "wasm plugin unloaded");
            Ok(())
        })
    }
}

fn map_instantiate_error(plugin_id: &str, e: &wasmtime::Error) -> WasmLoaderError {
    let reason = e.to_string();
    if reason.contains("infrarust:plugin/") {
        WasmLoaderError::CapabilityDenied {
            plugin_id: plugin_id.to_owned(),
            reason,
        }
    } else {
        WasmLoaderError::Instantiate {
            plugin_id: plugin_id.to_owned(),
            reason,
        }
    }
}

fn scan_wasm_files(dir: &Path) -> Result<Vec<PathBuf>, LoaderError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries =
            std::fs::read_dir(&current).map_err(|source| LoaderError::DirectoryNotAccessible {
                path: current.clone(),
                source,
            })?;
        for entry in entries {
            let entry = entry.map_err(|source| LoaderError::DirectoryNotAccessible {
                path: current.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|n| n.to_str()) != Some(CACHE_SUBDIR) {
                    stack.push(path);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn path_label(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>")
        .to_owned()
}
