//! Per-plugin store data: WASI context, resource table, capabilities, the captured
//! native plugin context, the resource-limiter backing, and epoch-control wiring.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Weak};

use infrarust_api::event::ListenerHandle;
use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;
use tokio::sync::Mutex;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::consts::{EPOCH_DEADLINE_TICKS, MAX_EPOCH_YIELDS_BEFORE_TRAP, MEMORY_LIMIT};
use crate::error::WasmLoaderError;
use crate::plugin::WasmInstance;

pub(crate) struct PluginStoreState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    capabilities: CapabilitySet,
    ctx: Option<Arc<dyn PluginContext>>,
    instance: Weak<Mutex<WasmInstance>>,
    poisoned: bool,
    pub(crate) plugin_id: String,
    pub(crate) epoch_yields: u32,
    next_listener_id: u64,
    listeners: HashMap<u64, ListenerHandle>,
}

impl PluginStoreState {
    pub(crate) fn reset_epoch_budget(&mut self) {
        self.epoch_yields = 0;
    }

    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }

    pub(crate) fn table_mut(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    pub(crate) fn ctx(&self) -> Option<&Arc<dyn PluginContext>> {
        self.ctx.as_ref()
    }

    pub(crate) fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    pub(crate) fn instance_ref(&self) -> Weak<Mutex<WasmInstance>> {
        self.instance.clone()
    }

    pub(crate) fn set_instance_ref(&mut self, instance: Weak<Mutex<WasmInstance>>) {
        self.instance = instance;
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub(crate) fn set_poisoned(&mut self) {
        self.poisoned = true;
    }

    pub(crate) fn mint_listener_id(&mut self) -> u64 {
        let id = self.next_listener_id;
        self.next_listener_id += 1;
        id
    }

    pub(crate) fn record_listener(&mut self, id: u64, handle: ListenerHandle) {
        self.listeners.insert(id, handle);
    }

    pub(crate) fn take_listener(&mut self, id: u64) -> Option<ListenerHandle> {
        self.listeners.remove(&id)
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
        instance: Weak::new(),
        poisoned: false,
        plugin_id,
        epoch_yields: 0,
        next_listener_id: 1,
        listeners: HashMap::new(),
    })
}

pub(crate) fn build_probe_state(plugin_id: String) -> PluginStoreState {
    PluginStoreState {
        table: ResourceTable::new(),
        wasi: WasiCtxBuilder::new().build(),
        limits: default_limits(),
        capabilities: CapabilitySet::default(),
        ctx: None,
        instance: Weak::new(),
        poisoned: false,
        plugin_id,
        epoch_yields: 0,
        next_listener_id: 1,
        listeners: HashMap::new(),
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
