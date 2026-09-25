use infrarust_api::filter::{FilterMetadata, FilterPriority};
use infrarust_api::permissions::Capability;

use crate::bindings::infrarust::plugin::codec_registry as wcr;
use crate::bindings::infrarust::plugin::types::ErrorKind;
use crate::codec::WasmCodecFilterFactory;
use crate::host_error::{HostResult, host_error};
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

impl wcr::Host for PluginStoreState {
    async fn register_codec_filter(
        &mut self,
        metadata: wcr::CodecFilterMetadata,
        factory: u64,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
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
            let registry = ctx.codec_filters().ok_or_else(|| {
                host_error(
                    ErrorKind::Unsupported,
                    "this proxy exposes no codec filter registry",
                )
            })?;
            registry.register(Box::new(WasmCodecFilterFactory::new(
                instantiator,
                factory,
                FilterMetadata {
                    id: metadata.id,
                    priority: priority_from_wit(metadata.priority),
                    after: metadata.after,
                    before: metadata.before,
                },
            )));
            Ok(())
        })())
    }

    async fn unregister_codec_filter(&mut self, id: String) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(
                Capability::CodecFilter,
                "codec-registry.unregister-codec-filter",
            )?;
            if let Ok(ctx) = self.services()
                && let Some(registry) = ctx.codec_filters()
            {
                registry.unregister(&id);
            }
            Ok(())
        })())
    }
}
