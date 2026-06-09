//! The synchronous linker for codec stores.
use wasmtime::Engine;
use wasmtime::component::{Component, Linker};

use super::store_state::CodecStoreState;
use crate::error::WasmLoaderError;

pub(crate) fn build_codec_linker(
    engine: &Engine,
    component: &Component,
    plugin_id: &str,
) -> Result<Linker<CodecStoreState>, WasmLoaderError> {
    let mut linker = Linker::<CodecStoreState>::new(engine);
    linker
        .define_unknown_imports_as_traps(component)
        .map_err(|e| WasmLoaderError::Instantiate {
            plugin_id: plugin_id.to_owned(),
            reason: format!("codec trap-fill linker: {e}"),
        })?;
    Ok(linker)
}
