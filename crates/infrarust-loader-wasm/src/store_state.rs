//! Per-plugin store data: WASI context, resource table, capabilities, the captured
//! native plugin context, the resource-limiter backing, and epoch-control wiring.

use std::path::Path;
use std::sync::Arc;

use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::consts::{EPOCH_DEADLINE_TICKS, MAX_EPOCH_YIELDS_BEFORE_TRAP, MEMORY_LIMIT};
use crate::error::WasmLoaderError;

pub(crate) struct PluginStoreState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    #[allow(dead_code)]
    capabilities: CapabilitySet,
    #[allow(dead_code)]
    ctx: Option<Arc<dyn PluginContext>>,
    pub(crate) plugin_id: String,
    pub(crate) epoch_yields: u32,
}

impl PluginStoreState {
    pub(crate) fn reset_epoch_budget(&mut self) {
        self.epoch_yields = 0;
    }

    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }
}

impl WasiView for PluginStoreState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn default_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .trap_on_grow_failure(true)
        .build()
}

fn build_wasi_ctx(data_dir: &Path) -> Result<WasiCtx, WasmLoaderError> {
    std::fs::create_dir_all(data_dir).map_err(|source| WasmLoaderError::WasiSetup {
        path: data_dir.to_path_buf(),
        source,
    })?;
    let mut builder = WasiCtxBuilder::new();
    builder
        .preopened_dir(data_dir, "/", DirPerms::all(), FilePerms::all())
        .map_err(|e| WasmLoaderError::WasiSetup {
            path: data_dir.to_path_buf(),
            source: std::io::Error::other(e.to_string()),
        })?;
    // TODO(WASM-2): widen with inherit_network()/extra preopens when the Network /
    // FilesystemExtended capabilities are granted.
    Ok(builder.build())
}

pub(crate) fn build_load_state(
    plugin_id: String,
    ctx: Arc<dyn PluginContext>,
    capabilities: CapabilitySet,
    data_dir: &Path,
) -> Result<PluginStoreState, WasmLoaderError> {
    Ok(PluginStoreState {
        table: ResourceTable::new(),
        wasi: build_wasi_ctx(data_dir)?,
        limits: default_limits(),
        capabilities,
        ctx: Some(ctx),
        plugin_id,
        epoch_yields: 0,
    })
}

pub(crate) fn build_probe_state(plugin_id: String) -> PluginStoreState {
    PluginStoreState {
        table: ResourceTable::new(),
        wasi: WasiCtxBuilder::new().build(),
        limits: default_limits(),
        capabilities: CapabilitySet::default(),
        ctx: None,
        plugin_id,
        epoch_yields: 0,
    }
}

pub(crate) fn install_epoch_control(store: &mut Store<PluginStoreState>) {
    store.set_epoch_deadline(EPOCH_DEADLINE_TICKS);
    store.epoch_deadline_callback(|mut ctx| {
        let state = ctx.data_mut();
        state.epoch_yields += 1;
        if state.epoch_yields > MAX_EPOCH_YIELDS_BEFORE_TRAP {
            tracing::warn!(
                plugin = %state.plugin_id,
                yields = state.epoch_yields,
                "wasm guest exceeded CPU budget — trapping"
            );
            Ok(UpdateDeadline::Interrupt)
        } else {
            Ok(UpdateDeadline::Yield(EPOCH_DEADLINE_TICKS))
        }
    });
}
