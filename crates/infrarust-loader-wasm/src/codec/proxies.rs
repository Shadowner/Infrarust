use std::sync::Arc;

use infrarust_api::event::ConnectionState;
use infrarust_api::filter::{
    CodecContext, CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict,
    FilterMetadata, FrameOutput,
};
use infrarust_api::types::RawPacket;
use wasmtime::Store;
use wasmtime::component::ResourceAny;

use super::CodecInstantiator;
use super::bindings::exports::infrarust::plugin::codec_filter::Guest as CodecGuest;
use super::convert;
use super::store_state::CodecStoreState;
use crate::consts::CODEC_EPOCH_DEADLINE_TICKS;
pub(crate) struct WasmCodecFilterFactory {
    instantiator: Arc<CodecInstantiator>,
    factory_id: u64,
    metadata: FilterMetadata,
}

impl WasmCodecFilterFactory {
    pub(crate) fn new(
        instantiator: Arc<CodecInstantiator>,
        factory_id: u64,
        metadata: FilterMetadata,
    ) -> Self {
        Self {
            instantiator,
            factory_id,
            metadata,
        }
    }
}

impl CodecFilterFactory for WasmCodecFilterFactory {
    fn metadata(&self) -> FilterMetadata {
        self.metadata.clone()
    }

    fn create(&self, init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        match self.instantiator.create_instance(self.factory_id, init) {
            Ok(instance) => Box::new(instance),
            Err(e) => {
                tracing::error!(
                    plugin = %self.instantiator.plugin_id(),
                    filter = %self.metadata.id,
                    error = %e,
                    "codec instance create failed; passing through for this connection"
                );
                Box::new(PassthroughInstance)
            }
        }
    }
}

pub(crate) struct WasmCodecFilterInstance {
    store: Store<CodecStoreState>,
    guest: CodecGuest,
    handle: ResourceAny,
    plugin_id: String,
    poisoned: bool,
}

impl WasmCodecFilterInstance {
    pub(crate) fn new(
        store: Store<CodecStoreState>,
        guest: CodecGuest,
        handle: ResourceAny,
        plugin_id: String,
    ) -> Self {
        Self {
            store,
            guest,
            handle,
            plugin_id,
            poisoned: false,
        }
    }

    fn poison(&mut self, op: &str, trap: &wasmtime::Error) {
        tracing::warn!(
            plugin = %self.plugin_id,
            op,
            error = %trap,
            "wasm codec filter trapped; passing through for this connection-side"
        );
        self.poisoned = true;
    }
}

impl CodecFilterInstance for WasmCodecFilterInstance {
    fn filter(
        &mut self,
        ctx: &CodecContext,
        packet: &mut RawPacket,
        output: &mut FrameOutput,
    ) -> CodecVerdict {
        if self.poisoned {
            return CodecVerdict::Pass;
        }
        self.store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
        let wit_ctx = convert::codec_context_to_wit(ctx);
        let wit_packet = crate::convert::raw_packet_to_wit(packet);
        match self
            .guest
            .filter_instance()
            .call_filter(&mut self.store, self.handle, &wit_ctx, &wit_packet)
        {
            Ok(out) => convert::apply_filter_output(out, packet, output),
            Err(trap) => {
                self.poison("filter", &trap);
                CodecVerdict::Pass
            }
        }
    }

    fn on_state_change(&mut self, new_state: ConnectionState) {
        if self.poisoned {
            return;
        }
        self.store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
        let wit_state = convert::connection_state_to_wit(new_state);
        if let Err(trap) =
            self.guest
                .filter_instance()
                .call_on_state_change(&mut self.store, self.handle, wit_state)
        {
            self.poison("on-state-change", &trap);
        }
    }

    fn on_compression_change(&mut self, threshold: i32) {
        if self.poisoned {
            return;
        }
        self.store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
        if let Err(trap) = self.guest.filter_instance().call_on_compression_change(
            &mut self.store,
            self.handle,
            threshold,
        ) {
            self.poison("on-compression-change", &trap);
        }
    }

    fn on_encryption_enabled(&mut self) {
        if self.poisoned {
            return;
        }
        self.store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
        if let Err(trap) = self
            .guest
            .filter_instance()
            .call_on_encryption_enabled(&mut self.store, self.handle)
        {
            self.poison("on-encryption-enabled", &trap);
        }
    }

    fn on_close(&mut self) {
        if !self.poisoned {
            self.store.set_epoch_deadline(CODEC_EPOCH_DEADLINE_TICKS);
            if let Err(trap) = self
                .guest
                .filter_instance()
                .call_on_close(&mut self.store, self.handle)
            {
                self.poison("on-close", &trap);
            }
        }
        // Release the guest-side resource (runs its destructor); the store drops next.
        let _ = self.handle.resource_drop(&mut self.store);
    }
}

struct PassthroughInstance;

impl CodecFilterInstance for PassthroughInstance {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        _packet: &mut RawPacket,
        _output: &mut FrameOutput,
    ) -> CodecVerdict {
        CodecVerdict::Pass
    }
}
