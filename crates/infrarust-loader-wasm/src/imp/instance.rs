use std::sync::Arc;

use infrarust_api::plugin::PluginContext;
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};

use crate::actor::InstanceRef;
use crate::bindings::{Plugin as PluginBindings, PluginPre};
use crate::config::SandboxLimits;
use crate::deadline::Deadline;
use crate::error::WasmLoaderError;
use crate::registrations::Registrations;
use crate::store_state::{PluginSetup, PluginStoreState, build_load_state, install_epoch_control};

pub(crate) struct LiveInstance {
    pub(crate) store: Store<PluginStoreState>,
    pub(crate) bindings: PluginBindings,
}

impl LiveInstance {
    pub(crate) fn begin_call(&mut self, deadline: Option<Deadline>) {
        self.store.data_mut().begin_call(deadline);
    }

    pub(crate) fn end_call(&mut self) {
        self.store.data_mut().end_call();
    }

    pub(crate) fn release_host_resources(&mut self) {
        self.store.data_mut().release_host_resources();
    }
}

pub(crate) struct InstanceFactory {
    engine: Engine,
    pre: PluginPre<PluginStoreState>,
    setup: PluginSetup,
}

impl InstanceFactory {
    pub(crate) fn new(
        engine: Engine,
        component: &Component,
        linker: &Linker<PluginStoreState>,
        setup: PluginSetup,
    ) -> Result<Self, WasmLoaderError> {
        let pre = linker
            .instantiate_pre(component)
            .and_then(PluginPre::new)
            .map_err(|e| map_instantiate_error(&setup.plugin_id, &e))?;
        Ok(Self { engine, pre, setup })
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.setup.plugin_id
    }

    pub(crate) fn sandbox(&self) -> &SandboxLimits {
        &self.setup.sandbox
    }

    pub(crate) fn ctx(&self) -> &Arc<dyn PluginContext> {
        &self.setup.ctx
    }

    pub(crate) fn registrations(&self) -> &Arc<Registrations> {
        &self.setup.registrations
    }

    pub(crate) async fn instantiate(
        &self,
        generation: u64,
        instance: InstanceRef,
    ) -> Result<LiveInstance, WasmLoaderError> {
        let state = build_load_state(&self.setup, generation, instance)?;
        let mut store = Store::new(&self.engine, state);
        install_epoch_control(&mut store, self.setup.sandbox.max_epoch_yields);
        store.limiter(|s: &mut PluginStoreState| {
            s.limits_mut() as &mut dyn wasmtime::ResourceLimiter
        });
        let bindings = self
            .pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| map_instantiate_error(&self.setup.plugin_id, &e))?;
        Ok(LiveInstance { store, bindings })
    }
}

pub(crate) fn map_instantiate_error(plugin_id: &str, e: &wasmtime::Error) -> WasmLoaderError {
    let reason = e.to_string();
    if reason.contains("infrarust:plugin/") {
        WasmLoaderError::CapabilityDenied {
            plugin_id: plugin_id.to_owned(),
            reason,
        }
    } else {
        WasmLoaderError::Instantiate {
            plugin_id: plugin_id.to_owned(),
            reason,
        }
    }
}
