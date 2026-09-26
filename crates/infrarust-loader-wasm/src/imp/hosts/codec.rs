use infrarust_api::filter::{FilterMetadata, FilterPriority};
use infrarust_api::permissions::Capability;

use crate::bindings::infrarust::plugin::codec_registry as wcr;
use crate::bindings::infrarust::plugin::types::{ErrorKind, HostError};
use crate::codec::WasmCodecFilterFactory;
use crate::host_error::{HostResult, filter_error, host_error};
use crate::store_state::PluginStoreState;

fn priority_from_wit(priority: wcr::FilterPriority) -> FilterPriority {
    match priority {
        wcr::FilterPriority::First => FilterPriority::First,
        wcr::FilterPriority::Early => FilterPriority::Early,
        wcr::FilterPriority::Normal => FilterPriority::Normal,
        wcr::FilterPriority::Late => FilterPriority::Late,
        wcr::FilterPriority::Last => FilterPriority::Last,
    }
}

fn no_registry() -> HostError {
    host_error(
        ErrorKind::Unsupported,
        "this proxy exposes no codec filter registry",
    )
}

impl wcr::Host for PluginStoreState {
    async fn register_codec_filter(
        &mut self,
        metadata: wcr::CodecFilterMetadata,
        factory: u64,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.add_codec_filter(metadata, factory))
    }

    async fn unregister_codec_filter(&mut self, id: String) -> wasmtime::Result<HostResult<()>> {
        Ok(self.remove_codec_filter(&id))
    }
}

impl PluginStoreState {
    fn add_codec_filter(
        &mut self,
        metadata: wcr::CodecFilterMetadata,
        factory: u64,
    ) -> HostResult<()> {
        self.check(
            Capability::CodecFilter,
            "codec-registry.register-codec-filter",
        )?;
        let instantiator = self.codec_instantiator().cloned().ok_or_else(|| {
            host_error(
                ErrorKind::Unavailable,
                "codec filters can only be registered by a loaded plugin",
            )
        })?;
        let ctx = self.services()?;
        let registry = ctx.codec_filters().ok_or_else(no_registry)?;
        let id = metadata.id.clone();
        let registered = registry.register(Box::new(WasmCodecFilterFactory::new(
            instantiator,
            factory,
            FilterMetadata {
                id: metadata.id,
                priority: priority_from_wit(metadata.priority),
                after: metadata.after,
                before: metadata.before,
            },
        )));
        match registered {
            Ok(()) => {
                self.registrations().record_codec_filter(&id);
                Ok(())
            }
            Err(error) => {
                self.report_codec_refusal(&id, &format!("refused, {error}"));
                Err(filter_error(&error))
            }
        }
    }

    fn remove_codec_filter(&mut self, id: &str) -> HostResult<()> {
        self.check(
            Capability::CodecFilter,
            "codec-registry.unregister-codec-filter",
        )?;
        let ctx = self.services()?;
        let registry = ctx.codec_filters().ok_or_else(no_registry)?;
        registry
            .unregister(id)
            .map_err(|error| filter_error(&error))?;
        self.registrations().forget_codec_filter(id);
        Ok(())
    }
}
