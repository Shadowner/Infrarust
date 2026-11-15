use wasmtime::Engine;
use wasmtime::component::{HasSelf, Linker};

use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, connection_claims, event_bus,
    events, limbo, load_balancer, log, messaging, permission_nodes, permissions, players,
    plugin_registry, providers, proxy_info, scheduler, server_manager, text,
};
use crate::error::WasmLoaderError;
use crate::store_state::PluginStoreState;

macro_rules! link {
    ($linker:expr, $iface:ident) => {
        $iface::add_to_linker::<_, HasSelf<_>>(&mut $linker, |state| state)
            .map_err(|e| link_err(concat!(stringify!($iface), " link failed"), &e))?;
    };
}

fn link_err(what: &str, e: &wasmtime::Error) -> WasmLoaderError {
    WasmLoaderError::Engine(wasmtime::format_err!("{what}: {e}"))
}

fn new_linker_with_wasi(engine: &Engine) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    let mut linker = Linker::<PluginStoreState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|e| link_err("wasi linker setup", &e))?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
        .map_err(|e| link_err("wasi-http linker setup", &e))?;
    Ok(linker)
}

pub(crate) fn build_linker(engine: &Engine) -> Result<Linker<PluginStoreState>, WasmLoaderError> {
    let mut linker = new_linker_with_wasi(engine)?;

    link!(linker, log);
    link!(linker, text);
    link!(linker, limbo);
    link!(linker, event_bus);
    link!(linker, events);
    link!(linker, players);
    link!(linker, command_manager);
    link!(linker, scheduler);
    link!(linker, config_service);
    link!(linker, server_manager);
    link!(linker, ban_service);
    link!(linker, codec_registry);
    link!(linker, load_balancer);
    link!(linker, messaging);
    link!(linker, proxy_info);
    link!(linker, plugin_registry);
    link!(linker, permissions);
    link!(linker, permission_nodes);
    link!(linker, providers);
    link!(linker, connection_claims);

    Ok(linker)
}
