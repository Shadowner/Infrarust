use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::event::ListenerHandle;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::scheduler::TaskHandle;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::actor::{CallKind, InstanceRef};
use crate::codec::CodecInstantiator;
use crate::config::SandboxLimits;
use crate::consts::{COMMAND_REFUSAL_BURST, DENIED_CALL_LOG_INTERVAL, EPOCH_DEADLINE_TICKS};
use crate::deadline::{Deadline, HostCallLimit};
use crate::error::WasmLoaderError;
use crate::rate_limit::RateLimit;
use crate::registrations::Registrations;

pub(crate) struct PluginSetup {
    pub(crate) plugin_id: String,
    pub(crate) ctx: Arc<dyn PluginContext>,
    pub(crate) capabilities: CapabilitySet,
    pub(crate) data_dir: PathBuf,
    pub(crate) codec: Option<Arc<CodecInstantiator>>,
    pub(crate) sandbox: SandboxLimits,
    pub(crate) registrations: Arc<Registrations>,
}

pub(crate) struct PluginStoreState {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: StoreLimits,
    capabilities: CapabilitySet,
    ctx: Option<Arc<dyn PluginContext>>,
    instance: InstanceRef,
    deadline: Option<Deadline>,
    pub(crate) plugin_id: String,
    pub(crate) epoch_yields: u32,
    host_call_timeout: Duration,
    generation: u64,
    registrations: Arc<Registrations>,
    next_listener_id: u64,
    listeners: HashMap<u64, ListenerHandle>,
    tasks: HashSet<u64>,
    codec: Option<Arc<CodecInstantiator>>,
    denials: HashMap<Capability, RateLimit>,
    command_refusals: RateLimit,
}

impl PluginStoreState {
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

    pub(crate) fn report_denied(&mut self, capability: Capability, call: &'static str) {
        let Some(suppressed) = self
            .denials
            .entry(capability)
            .or_insert_with(|| RateLimit::new(DENIED_CALL_LOG_INTERVAL, 1))
            .admit(Instant::now())
        else {
            return;
        };
        let name = capability.to_kebab();
        if capability == Capability::Limbo {
            tracing::error!(plugin = %self.plugin_id, call, capability = name, suppressed,
                "wasm plugin call refused: missing capability `{name}`; the call did nothing");
        } else {
            tracing::warn!(plugin = %self.plugin_id, call, capability = name, suppressed,
                "wasm plugin call refused: missing capability `{name}`");
        }
    }

    pub(crate) fn report_command_refusal(&mut self, name: &str, reason: &str) {
        let Some(suppressed) = self.command_refusals.admit(Instant::now()) else {
            return;
        };
        tracing::warn!(plugin = %self.plugin_id, command = name, suppressed,
            "wasm plugin command registration: {reason}");
    }

    pub(crate) fn instance_ref(&self, kind: CallKind) -> InstanceRef {
        self.instance.for_calls(kind)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn registrations(&self) -> &Arc<Registrations> {
        &self.registrations
    }

    pub(crate) fn begin_call(&mut self, deadline: Option<Deadline>) {
        self.epoch_yields = 0;
        self.deadline = deadline;
    }

    pub(crate) fn end_call(&mut self) {
        self.deadline = None;
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

    pub(crate) fn record_task(&mut self, handle: u64) {
        self.tasks.insert(handle);
    }

    pub(crate) fn forget_task(&mut self, handle: u64) {
        self.tasks.remove(&handle);
    }

    pub(crate) fn release_host_resources(&mut self) {
        let listeners: Vec<ListenerHandle> = self.listeners.drain().map(|(_, h)| h).collect();
        let tasks: Vec<u64> = self.tasks.drain().collect();
        let Some(ctx) = self.ctx.as_ref() else {
            return;
        };
        for handle in listeners {
            ctx.event_bus().unsubscribe(handle);
        }
        for task in tasks {
            ctx.scheduler().cancel(TaskHandle::new(task));
        }
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
    setup: &PluginSetup,
    generation: u64,
    instance: InstanceRef,
) -> Result<PluginStoreState, WasmLoaderError> {
    Ok(PluginStoreState {
        table: ResourceTable::new(),
        wasi: build_wasi_ctx(&setup.data_dir)?,
        limits: store_limits(&setup.sandbox),
        capabilities: setup.capabilities.clone(),
        ctx: Some(Arc::clone(&setup.ctx)),
        instance,
        deadline: None,
        plugin_id: setup.plugin_id.clone(),
        epoch_yields: 0,
        host_call_timeout: setup.sandbox.host_call_timeout,
        generation,
        registrations: Arc::clone(&setup.registrations),
        next_listener_id: 1,
        listeners: HashMap::new(),
        tasks: HashSet::new(),
        codec: setup.codec.clone(),
        denials: HashMap::new(),
        command_refusals: RateLimit::new(DENIED_CALL_LOG_INTERVAL, COMMAND_REFUSAL_BURST),
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
        deadline: None,
        plugin_id,
        epoch_yields: 0,
        host_call_timeout: sandbox.host_call_timeout,
        generation: 0,
        registrations: Arc::default(),
        next_listener_id: 1,
        listeners: HashMap::new(),
        tasks: HashSet::new(),
        codec: None,
        denials: HashMap::new(),
        command_refusals: RateLimit::new(DENIED_CALL_LOG_INTERVAL, COMMAND_REFUSAL_BURST),
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
