pub(crate) mod bindings;
mod convert;
mod linker;
mod proxies;
mod store_state;

use infrarust_api::filter::CodecSessionInit;
use wasmtime::component::{Component, InstancePre};
use wasmtime::{Engine, Store};

pub(crate) use proxies::WasmCodecFilterFactory;

use bindings::exports::infrarust::plugin::codec_filter::GuestIndices;
use linker::build_codec_linker;
use proxies::WasmCodecFilterInstance;
use store_state::CodecStoreState;

use crate::consts::CODEC_EPOCH_DEADLINE_TICKS;
use crate::error::WasmLoaderError;

pub(crate) struct CodecInstantiator {
    engine: Engine,
    pre: InstancePre<CodecStoreState>,
    indices: GuestIndices,
    plugin_id: String,
}

impl CodecInstantiator {
    pub(crate) fn new(
        engine: Engine,
        component: &Component,
        plugin_id: String,
    ) -> Result<Self, WasmLoaderError> {
        let linker = build_codec_linker(&engine, component, &plugin_id)?;
        let pre = linker
            .instantiate_pre(component)
            .map_err(|e| instantiate_err(&plugin_id, "instantiate_pre", &e))?;
        let indices =
            GuestIndices::new(&pre).map_err(|e| instantiate_err(&plugin_id, "guest-indices", &e))?;
        Ok(Self {
            engine,
            pre,
            indices,
            plugin_id,
        })
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    fn create_instance(
        &self,
        factory_id: u64,
        init: &CodecSessionInit,
    ) -> Result<WasmCodecFilterInstance, WasmLoaderError> {
        let mut store = Store::new(&self.engine, CodecStoreState);
        store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
        let instance = self
            .pre
            .instantiate(&mut store)
            .map_err(|e| instantiate_err(&self.plugin_id, "instantiate", &e))?;
        let guest = self
            .indices
            .load(&mut store, &instance)
            .map_err(|e| instantiate_err(&self.plugin_id, "guest-load", &e))?;
        let wit_init = convert::session_init_to_wit(init);
        let handle = guest
            .call_create(&mut store, factory_id, &wit_init)
            .map_err(|e| instantiate_err(&self.plugin_id, "create", &e))?;
        Ok(WasmCodecFilterInstance::new(
            store,
            guest,
            handle,
            self.plugin_id.clone(),
        ))
    }
}

fn instantiate_err(plugin_id: &str, what: &str, e: &wasmtime::Error) -> WasmLoaderError {
    WasmLoaderError::Instantiate {
        plugin_id: plugin_id.to_owned(),
        reason: format!("codec {what}: {e}"),
    }
}
