//! The synchronous linker for codec stores.
use wasmtime::Engine;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker, LinkerInstance, ResourceType};

use super::host;
use super::store_state::CodecStoreState;
use crate::error::WasmLoaderError;

pub(crate) fn build_codec_linker(
    engine: &Engine,
    component: &Component,
    plugin_id: &str,
) -> Result<Linker<CodecStoreState>, WasmLoaderError> {
    let mut linker = Linker::<CodecStoreState>::new(engine);
    define_imports(&mut linker, engine, component).map_err(|e| WasmLoaderError::Instantiate {
        plugin_id: plugin_id.to_owned(),
        reason: format!("codec linker: {e}"),
    })?;
    Ok(linker)
}

fn define_imports(
    linker: &mut Linker<CodecStoreState>,
    engine: &Engine,
    component: &Component,
) -> wasmtime::Result<()> {
    let ty = component.component_type();
    for (name, item) in ty.imports(engine) {
        match item {
            ComponentItem::ComponentInstance(instance) => {
                let interface = name.split_once('@').map_or(name, |(path, _)| path);
                let mut slot = linker.instance(name)?;
                for (export, item) in instance.exports(engine) {
                    define_item(&mut slot, name, interface, export, &item)?;
                }
            }
            ComponentItem::Module(_) | ComponentItem::Component(_) => {
                wasmtime::bail!("unable to define import `{name}` for a codec filter")
            }
            item => define_item(&mut linker.root(), name, "", name, &item)?,
        }
    }
    Ok(())
}

fn define_item(
    slot: &mut LinkerInstance<'_, CodecStoreState>,
    import: &str,
    interface: &str,
    export: &str,
    item: &ComponentItem,
) -> wasmtime::Result<()> {
    match item {
        ComponentItem::ComponentFunc(_) => {
            if !host::define(slot, interface, export)? {
                let unavailable = if import == export {
                    export.to_owned()
                } else {
                    format!("{import}#{export}")
                };
                slot.func_new(export, move |_, _, _, _| {
                    wasmtime::bail!("`{unavailable}` is not available to codec filters")
                })?;
            }
        }
        ComponentItem::Resource(_) => {
            slot.resource(export, ResourceType::host::<()>(), |_, _| Ok(()))?;
        }
        _ => {}
    }
    Ok(())
}
