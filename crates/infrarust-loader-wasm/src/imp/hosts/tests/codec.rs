use std::collections::HashMap;
use std::sync::Arc;

use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FrameOutput,
};
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::types::RawPacket;
use infrarust_core::filter::FilterOwner;
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions};

use crate::bindings::infrarust::plugin::{codec_registry, types as wt};
use crate::config::SandboxLimits;
use crate::store_state::{PluginStoreState, build_probe_state};

use super::services;

struct PassFactory(&'static str);

struct Pass;

impl CodecFilterFactory for PassFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(self.0)
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(Pass)
    }
}

impl CodecFilterInstance for Pass {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

fn codec_state(factory: &PluginContextFactoryImpl, plugin_id: &str) -> PluginStoreState {
    build_probe_state(plugin_id.to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::baseline().with(Capability::CodecFilter))
        .with_ctx(factory.create_context(plugin_id))
}

#[tokio::test]
async fn a_codec_filter_can_only_be_unregistered_by_the_plugin_that_owns_it() {
    let registry = Arc::new(CodecFilterRegistryImpl::new());
    let grant = |plugin_id: &str| {
        let permissions = PluginPermissions {
            permissions: vec![Capability::CodecFilter.to_kebab().to_owned()],
            ..PluginPermissions::default()
        };
        (plugin_id.to_owned(), permissions)
    };
    let factory = PluginContextFactoryImpl::new(
        PluginServices {
            codec_filter_registry: Arc::clone(&registry),
            ..services(vec![])
        },
        HashMap::from([grant("owner"), grant("intruder")]),
    );
    let mut owner = codec_state(&factory, "owner");
    let mut intruder = codec_state(&factory, "intruder");
    owner
        .ctx()
        .and_then(|ctx| ctx.codec_filters())
        .expect("the owner holds the codec-filter capability")
        .register(Box::new(PassFactory("shared")))
        .unwrap();

    let refused = codec_registry::Host::unregister_codec_filter(&mut intruder, "shared".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::Conflict);
    assert!(refused.message.contains("owner"), "{}", refused.message);
    assert_eq!(
        registry.owner_of("shared"),
        Some(FilterOwner::plugin("owner"))
    );
    let missing = codec_registry::Host::unregister_codec_filter(&mut intruder, "missing".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.kind, wt::ErrorKind::NotFound);

    assert_eq!(
        codec_registry::Host::unregister_codec_filter(&mut owner, "shared".into())
            .await
            .unwrap(),
        Ok(())
    );
    assert!(registry.is_empty());
}
