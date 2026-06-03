use std::path::Path;

use infrarust_api::event::BoxFuture;
use infrarust_api::loader::{LoaderError, PluginContextFactory, PluginLoader};
use infrarust_api::plugin::{Plugin, PluginMetadata};
use wasmtime::Engine;

pub struct WasmPluginLoader {
    #[allow(dead_code)] // used once discovery/instantiation lands
    engine: Engine,
}

impl WasmPluginLoader {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}

impl PluginLoader for WasmPluginLoader {
    fn name(&self) -> &str {
        "wasm"
    }

    fn discover<'a>(
        &'a self,
        _plugin_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Vec<PluginMetadata>, LoaderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn load<'a>(
        &'a self,
        plugin_id: &'a str,
        _context_factory: &'a dyn PluginContextFactory,
    ) -> BoxFuture<'a, Result<Box<dyn Plugin>, LoaderError>> {
        Box::pin(async move {
            Err(LoaderError::PluginNotFound {
                plugin_id: plugin_id.to_owned(),
            })
        })
    }

    fn unload<'a>(&'a self, _plugin_id: &'a str) -> BoxFuture<'a, Result<(), LoaderError>> {
        Box::pin(async { Ok(()) })
    }
}
