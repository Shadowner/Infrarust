use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::ListenerHandle;
use infrarust_api::permissions::CapabilitySet;
use infrarust_api::plugin::PluginContext;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::actor::{CallKind, InstanceRef};
use crate::codec::CodecInstantiator;
use crate::config::SandboxLimits;
use crate::consts::EPOCH_DEADLINE_TICKS;
use crate::deadline::{Deadline, HostCallLimit};
use crate::error::WasmLoaderError;

pub(crate) struct PluginStoreState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    capabilities: CapabilitySet,
    ctx: Option<Arc<dyn PluginContext>>,
    instance: InstanceRef,
    poisoned: bool,
    call_in_flight: Option<&'static str>,
    deadline: Option<Deadline>,
    pub(crate) plugin_id: String,
    pub(crate) epoch_yields: u32,
    host_call_timeout: Duration,
    next_listener_id: u64,
    listeners: HashMap<u64, ListenerHandle>,
    codec: Option<Arc<CodecInstantiator>>,
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

    pub(crate) fn require_ctx(&self) -> wasmtime::Result<Arc<dyn PluginContext>> {
        self.ctx.clone().ok_or_else(|| {
            wasmtime::Error::msg(
                "plugin context unavailable (host function called off the load path)",
            )
        })
    }

    pub(crate) fn host_call_timeout(&self) -> Duration {
        self.host_call_timeout
    }

    pub(crate) fn host_call_limit(&self, timeout: Duration) -> HostCallLimit {
        HostCallLimit::new(timeout, self.deadline)
    }

    pub(crate) fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    pub(crate) fn instance_ref(&self, kind: CallKind) -> InstanceRef {
        self.instance.for_calls(kind)
    }

    pub(crate) fn set_instance_ref(&mut self, instance: InstanceRef) {
        self.instance = instance;
    }

    pub(crate) fn is_poisoned(&mut self) -> bool {
        if let Some(op) = self.call_in_flight.take() {
            tracing::error!(plugin = %self.plugin_id, op,
                "previous wasm guest call was abandoned mid-execution; poisoning instance");
            self.poisoned = true;
        }
        self.poisoned
    }

    pub(crate) fn begin_call(&mut self, op: &'static str, deadline: Option<Deadline>) {
        self.call_in_flight = Some(op);
        self.deadline = deadline;
    }

    pub(crate) fn end_call(&mut self) {
        self.call_in_flight = None;
        self.deadline = None;
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

    pub(crate) fn codec_instantiator(&self) -> Option<&Arc<CodecInstantiator>> {
        self.codec.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn with_capabilities(mut self, capabilities: CapabilitySet) -> Self {
        self.capabilities = capabilities;
        self
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

fn store_limits(sandbox: &SandboxLimits) -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(sandbox.memory_bytes)
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
    Ok(builder.build())
}

pub(crate) fn build_load_state(
    plugin_id: String,
    ctx: Arc<dyn PluginContext>,
    capabilities: CapabilitySet,
    data_dir: &Path,
    codec: Option<Arc<CodecInstantiator>>,
    sandbox: &SandboxLimits,
) -> Result<PluginStoreState, WasmLoaderError> {
    Ok(PluginStoreState {
        table: ResourceTable::new(),
        wasi: build_wasi_ctx(data_dir)?,
        limits: store_limits(sandbox),
        capabilities,
        ctx: Some(ctx),
        instance: InstanceRef::detached(),
        poisoned: false,
        call_in_flight: None,
        deadline: None,
        plugin_id,
        epoch_yields: 0,
        host_call_timeout: sandbox.host_call_timeout,
        next_listener_id: 1,
        listeners: HashMap::new(),
        codec,
    })
}

pub(crate) fn build_probe_state(plugin_id: String, sandbox: &SandboxLimits) -> PluginStoreState {
    PluginStoreState {
        table: ResourceTable::new(),
        wasi: WasiCtxBuilder::new().build(),
        limits: store_limits(sandbox),
        capabilities: CapabilitySet::default(),
        ctx: None,
        instance: InstanceRef::detached(),
        poisoned: false,
        call_in_flight: None,
        deadline: None,
        plugin_id,
        epoch_yields: 0,
        host_call_timeout: sandbox.host_call_timeout,
        next_listener_id: 1,
        listeners: HashMap::new(),
        codec: None,
    }
}

pub(crate) fn install_epoch_control(store: &mut Store<PluginStoreState>, max_epoch_yields: u32) {
    store.set_epoch_deadline(EPOCH_DEADLINE_TICKS);
    store.epoch_deadline_callback(move |mut ctx| {
        let state = ctx.data_mut();
        state.epoch_yields += 1;
        if state.epoch_yields > max_epoch_yields {
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
