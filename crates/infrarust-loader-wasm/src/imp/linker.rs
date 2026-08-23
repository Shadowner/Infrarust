//! Builds the per-plugin host linker.
//! `log` is always linked. `types` defines no functions, so it has no linker entry.

use infrarust_api::permissions::{Capability, CapabilitySet};
use wasmtime::Engine;
use wasmtime::component::{HasSelf, Linker};

use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, log,
    player_registry, scheduler, server_manager,
};
use crate::error::WasmLoaderError;
use crate::store_state::PluginStoreState;

macro_rules! link {
    ($linker:expr, $plugin_id:expr, $iface:ident) => {
        $iface::add_to_linker::<_, HasSelf<_>>(&mut $linker, |state| state).map_err(|e| {
            instantiate_err(
                $plugin_id,
                &format!("{} link failed", stringify!($iface)),
                e,
            )
        })?;
    };
}

fn instantiate_err(plugin_id: &str, what: &str, e: wasmtime::Error) -> WasmLoaderError {
    WasmLoaderError::Instantiate {
        plugin_id: plugin_id.to_owned(),
        reason: format!("{what}: {e}"),
    }
}

fn new_linker_with_wasi(
    engine: &Engine,
    plugin_id: &str,
) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    let mut linker = Linker::<PluginStoreState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|e| instantiate_err(plugin_id, "wasi linker setup", e))?;
    Ok(linker)
}

pub(crate) fn build_linker(
    engine: &Engine,
    plugin_id: &str,
    caps: &CapabilitySet,
) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    let mut linker = new_linker_with_wasi(engine, plugin_id)?;

    link!(linker, plugin_id, log); // always available
    link!(linker, plugin_id, limbo);

    if caps.has(Capability::EventBus) {
        link!(linker, plugin_id, event_bus);
    }
    if caps.has(Capability::PlayerRead) {
        link!(linker, plugin_id, player_registry);
    }
    if caps.has(Capability::Command) {
        link!(linker, plugin_id, command_manager);
    }
    if caps.has(Capability::Scheduler) {
        link!(linker, plugin_id, scheduler);
    }
    if caps.has(Capability::ConfigRead) {
        link!(linker, plugin_id, config_service);
    }
    if caps.has(Capability::ServerManage) {
        link!(linker, plugin_id, server_manager);
    }
    if caps.has(Capability::Ban) {
        link!(linker, plugin_id, ban_service);
    }
    if caps.has(Capability::CodecFilter) {
        link!(linker, plugin_id, codec_registry);
    }

    Ok(linker)
}

pub(crate) fn build_probe_linker(
    engine: &Engine,
    plugin_id: &str,
) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    build_linker(engine, plugin_id, &CapabilitySet::native_trusted())
}
