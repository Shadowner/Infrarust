use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::event::ListenerHandle;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::player::BossBarHandle;
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::scheduler::TaskHandle;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::actor::{CallKind, InstanceRef};
use crate::codec::CodecInstantiator;
use crate::config::SandboxLimits;
use crate::consts::{
    CODEC_REFUSAL_BURST, COMMAND_REFUSAL_BURST, DENIED_CALL_LOG_INTERVAL, EPOCH_DEADLINE_TICKS,
};
use crate::deadline::{Deadline, HostCallLimit};
use crate::error::WasmLoaderError;
use crate::host_error::{HostResult, no_services};
use crate::mounts::Mount;
use crate::network::{HttpHooks, NetworkPolicy, probe_policy};
use crate::rate_limit::RateLimit;
use crate::registrations::Registrations;
use crate::sync::lock;

pub(crate) struct PluginSetup {
    pub(crate) plugin_id: String,
    pub(crate) ctx: Arc<dyn PluginContext>,
    pub(crate) capabilities: CapabilitySet,
    pub(crate) data_dir: PathBuf,
    pub(crate) codec: Option<Arc<CodecInstantiator>>,
    pub(crate) sandbox: SandboxLimits,
    pub(crate) registrations: Arc<Registrations>,
    pub(crate) network: Arc<NetworkPolicy>,
    pub(crate) mounts: Arc<[Mount]>,
}

struct Sandbox {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
    http_hooks: HttpHooks,
    limits: StoreLimits,
    deadline: Option<Deadline>,
    epoch_yields: u32,
}

impl Sandbox {
    fn new(wasi: WasiCtx, network: Arc<NetworkPolicy>, limits: &SandboxLimits) -> Self {
        Self {
            table: ResourceTable::new(),
            wasi,
            http: WasiHttpCtx::new(),
            http_hooks: HttpHooks::new(network, limits.host_call_timeout),
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .trap_on_grow_failure(true)
                .build(),
            deadline: None,
            epoch_yields: 0,
        }
    }
}

struct Guest {
    plugin_id: String,
    generation: u64,
    capabilities: CapabilitySet,
    ctx: Option<Arc<dyn PluginContext>>,
    instance: Option<InstanceRef>,
    registrations: Arc<Registrations>,
    codec: Option<Arc<CodecInstantiator>>,
    host_call_timeout: Duration,
}

#[derive(Default)]
pub(crate) struct LiveTasks {
    slots: Mutex<HashMap<u64, Option<TaskHandle>>>,
}

impl LiveTasks {
    fn reserve(&self, id: u64) {
        lock(&self.slots).insert(id, None);
    }

    fn bind(&self, id: u64, handle: TaskHandle) {
        if let Some(slot) = lock(&self.slots).get_mut(&id) {
            *slot = Some(handle);
        }
    }

    pub(crate) fn fired(&self, id: u64) {
        lock(&self.slots).remove(&id);
    }

    fn take(&self, id: u64) -> Option<TaskHandle> {
        lock(&self.slots).remove(&id).flatten()
    }

    fn drain(&self) -> Vec<TaskHandle> {
        lock(&self.slots)
            .drain()
            .filter_map(|(_, handle)| handle)
            .collect()
    }
}

struct HostResources {
    next_listener_id: u64,
    listeners: HashMap<u64, Vec<ListenerHandle>>,
    next_task_id: u64,
    tasks: Arc<LiveTasks>,
    boss_bars: HashMap<uuid::Uuid, BossBarHandle>,
}

impl HostResources {
    fn new() -> Self {
        Self {
            next_listener_id: 1,
            listeners: HashMap::new(),
            next_task_id: 1,
            tasks: Arc::default(),
            boss_bars: HashMap::new(),
        }
    }
}

struct Throttles {
    denials: HashMap<Capability, RateLimit>,
    command_refusals: RateLimit,
    codec_refusals: RateLimit,
}

impl Throttles {
    fn new() -> Self {
        Self {
            denials: HashMap::new(),
            command_refusals: RateLimit::new(DENIED_CALL_LOG_INTERVAL, COMMAND_REFUSAL_BURST),
            codec_refusals: RateLimit::new(DENIED_CALL_LOG_INTERVAL, CODEC_REFUSAL_BURST),
        }
    }
}

pub(crate) struct PluginStoreState {
    sandbox: Sandbox,
    guest: Guest,
    resources: HostResources,
    throttles: Throttles,
}

impl PluginStoreState {
    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.sandbox.limits
    }

    pub(crate) fn table_mut(&mut self) -> &mut ResourceTable {
        &mut self.sandbox.table
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.guest.plugin_id
    }

    pub(crate) fn ctx(&self) -> Option<&Arc<dyn PluginContext>> {
        self.guest.ctx.as_ref()
    }

    pub(crate) fn host_call_timeout(&self) -> Duration {
        self.guest.host_call_timeout
    }

    pub(crate) fn host_call_limit(&self, timeout: Duration) -> HostCallLimit {
        HostCallLimit::new(timeout, self.sandbox.deadline)
    }

    pub(crate) fn capabilities(&self) -> &CapabilitySet {
        &self.guest.capabilities
    }

    pub(crate) fn report_denied(&mut self, capability: Capability, call: fmt::Arguments<'_>) {
        let Some(suppressed) = self
            .throttles
            .denials
            .entry(capability)
            .or_insert_with(|| RateLimit::new(DENIED_CALL_LOG_INTERVAL, 1))
            .admit(Instant::now())
        else {
            return;
        };
        let name = capability.to_kebab();
        if capability == Capability::Limbo {
            tracing::error!(plugin = %self.guest.plugin_id, %call, capability = name, suppressed,
                "wasm plugin call refused: missing capability `{name}`; the call did nothing");
        } else {
            tracing::warn!(plugin = %self.guest.plugin_id, %call, capability = name, suppressed,
                "wasm plugin call refused: missing capability `{name}`");
        }
    }

    pub(crate) fn report_command_refusal(&mut self, name: &str, reason: &str) {
        let Some(suppressed) = self.throttles.command_refusals.admit(Instant::now()) else {
            return;
        };
        tracing::warn!(plugin = %self.guest.plugin_id, command = name, suppressed,
            "wasm plugin command registration: {reason}");
    }

    pub(crate) fn report_codec_refusal(&mut self, filter: &str, reason: &str) {
        let Some(suppressed) = self.throttles.codec_refusals.admit(Instant::now()) else {
            return;
        };
        tracing::warn!(plugin = %self.guest.plugin_id, filter, suppressed,
            "wasm plugin codec filter registration: {reason}");
    }

    pub(crate) fn instance_ref(&self, kind: CallKind) -> HostResult<InstanceRef> {
        self.guest
            .instance
            .as_ref()
            .map(|instance| instance.for_calls(kind))
            .ok_or_else(no_services)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.guest.generation
    }

    pub(crate) fn registrations(&self) -> &Arc<Registrations> {
        &self.guest.registrations
    }

    pub(crate) fn begin_call(&mut self, deadline: Option<Deadline>) {
        self.sandbox.epoch_yields = 0;
        self.sandbox.deadline = deadline;
    }

    pub(crate) fn end_call(&mut self) {
        self.sandbox.deadline = None;
    }

    pub(crate) fn mint_listener_id(&mut self) -> u64 {
        let id = self.resources.next_listener_id;
        self.resources.next_listener_id += 1;
        id
    }

    pub(crate) fn record_listener(&mut self, id: u64, handles: Vec<ListenerHandle>) {
        self.resources.listeners.insert(id, handles);
    }

    pub(crate) fn take_listener(&mut self, id: u64) -> Option<Vec<ListenerHandle>> {
        self.resources.listeners.remove(&id)
    }

    pub(crate) fn boss_bar_count(&self) -> usize {
        self.resources.boss_bars.len()
    }

    pub(crate) fn record_boss_bar(&mut self, handle: BossBarHandle) {
        self.resources.boss_bars.insert(handle.id(), handle);
    }

    pub(crate) fn boss_bar(&self, id: uuid::Uuid) -> Option<&BossBarHandle> {
        self.resources.boss_bars.get(&id)
    }

    pub(crate) fn forget_boss_bar(&mut self, id: uuid::Uuid) -> Option<BossBarHandle> {
        self.resources.boss_bars.remove(&id)
    }

    pub(crate) fn reserve_task(&mut self) -> (u64, Arc<LiveTasks>) {
        let id = self.resources.next_task_id;
        self.resources.next_task_id += 1;
        self.resources.tasks.reserve(id);
        (id, Arc::clone(&self.resources.tasks))
    }

    pub(crate) fn bind_task(&mut self, id: u64, handle: TaskHandle) {
        self.resources.tasks.bind(id, handle);
    }

    pub(crate) fn take_task(&mut self, id: u64) -> Option<TaskHandle> {
        self.resources.tasks.take(id)
    }

    pub(crate) fn release_host_resources(&mut self) {
        let resources = &mut self.resources;
        let listeners: Vec<ListenerHandle> =
            resources.listeners.drain().flat_map(|(_, h)| h).collect();
        let tasks = resources.tasks.drain();
        for (_, bar) in resources.boss_bars.drain() {
            let _ = bar.hide();
        }
        let Some(ctx) = self.guest.ctx.as_ref() else {
            return;
        };
        for handle in listeners {
            ctx.event_bus().unsubscribe(handle);
        }
        for task in tasks {
            ctx.scheduler().cancel(task);
        }
    }

    pub(crate) fn codec_instantiator(&self) -> Option<&Arc<CodecInstantiator>> {
        self.guest.codec.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn with_capabilities(mut self, capabilities: CapabilitySet) -> Self {
        self.guest.capabilities = capabilities;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_ctx(mut self, ctx: Arc<dyn PluginContext>) -> Self {
        self.guest.ctx = Some(ctx);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_instance(mut self, instance: InstanceRef) -> Self {
        self.guest.instance = Some(instance);
        self
    }
}

impl WasiView for PluginStoreState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.sandbox.wasi,
            table: &mut self.sandbox.table,
        }
    }
}

impl WasiHttpView for PluginStoreState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.sandbox.http,
            table: &mut self.sandbox.table,
            hooks: &mut self.sandbox.http_hooks,
        }
    }
}

fn build_wasi_ctx(
    data_dir: &Path,
    network: &Arc<NetworkPolicy>,
    mounts: &[Mount],
) -> Result<WasiCtx, WasmLoaderError> {
    std::fs::create_dir_all(data_dir).map_err(|source| WasmLoaderError::WasiSetup {
        path: data_dir.to_path_buf(),
        source,
    })?;
    let mut builder = WasiCtxBuilder::new();
    builder
        .preopened_dir(data_dir, "/", FsPerms::ReadWrite)
        .map_err(|e| WasmLoaderError::WasiSetup {
            path: data_dir.to_path_buf(),
            source: std::io::Error::other(e),
        })?;
    for mount in mounts {
        builder
            .preopened_dir(&mount.host, &mount.guest, mount.perms())
            .map_err(|e| WasmLoaderError::WasiSetup {
                path: mount.host.clone(),
                source: std::io::Error::other(e),
            })?;
    }
    builder
        .allow_tcp(true)
        .allow_udp(true)
        .socket_addr_check(network.socket_check())
        .allow_ip_name_lookup(network.dns());
    Ok(builder.build())
}

pub(crate) fn build_load_state(
    setup: &PluginSetup,
    generation: u64,
    instance: InstanceRef,
) -> Result<PluginStoreState, WasmLoaderError> {
    let wasi = build_wasi_ctx(&setup.data_dir, &setup.network, &setup.mounts)?;
    setup.network.warm_in_background();
    Ok(PluginStoreState {
        sandbox: Sandbox::new(wasi, Arc::clone(&setup.network), &setup.sandbox),
        guest: Guest {
            plugin_id: setup.plugin_id.clone(),
            generation,
            capabilities: setup.capabilities.clone(),
            ctx: Some(Arc::clone(&setup.ctx)),
            instance: Some(instance),
            registrations: Arc::clone(&setup.registrations),
            codec: setup.codec.clone(),
            host_call_timeout: setup.sandbox.host_call_timeout,
        },
        resources: HostResources::new(),
        throttles: Throttles::new(),
    })
}

pub(crate) fn build_probe_state(plugin_id: String, sandbox: &SandboxLimits) -> PluginStoreState {
    PluginStoreState {
        sandbox: Sandbox::new(
            WasiCtxBuilder::new().build(),
            probe_policy(plugin_id.clone()),
            sandbox,
        ),
        guest: Guest {
            plugin_id,
            generation: 0,
            capabilities: CapabilitySet::default(),
            ctx: None,
            instance: None,
            registrations: Arc::default(),
            codec: None,
            host_call_timeout: sandbox.host_call_timeout,
        },
        resources: HostResources::new(),
        throttles: Throttles::new(),
    }
}

pub(crate) fn install_epoch_control(store: &mut Store<PluginStoreState>, max_epoch_yields: u32) {
    store.set_epoch_deadline(EPOCH_DEADLINE_TICKS);
    store.epoch_deadline_callback(move |mut ctx| {
        let state = ctx.data_mut();
        state.sandbox.epoch_yields += 1;
        if state.sandbox.epoch_yields > max_epoch_yields {
            tracing::warn!(
                plugin = %state.guest.plugin_id,
                yields = state.sandbox.epoch_yields,
                "wasm guest exceeded CPU budget — trapping"
            );
            Ok(UpdateDeadline::Interrupt)
        } else {
            Ok(UpdateDeadline::Yield(EPOCH_DEADLINE_TICKS))
        }
    });
}
