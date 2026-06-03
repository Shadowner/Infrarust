//! Builds the per-plugin host linker.
use wasmtime::Engine;
use wasmtime::component::Linker;

use crate::error::WasmLoaderError;
use crate::store_state::PluginStoreState;

pub(crate) fn build_linker(
    engine: &Engine,
    plugin_id: &str,
) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    let mut linker = Linker::<PluginStoreState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|e| {
        WasmLoaderError::Instantiate {
            plugin_id: plugin_id.to_owned(),
            reason: format!("wasi linker setup failed: {e}"),
        }
    })?;
    Ok(linker)
}
