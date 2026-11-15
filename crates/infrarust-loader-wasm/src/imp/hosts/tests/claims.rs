use std::collections::HashMap;
use std::sync::Arc;

use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions};

use crate::bindings::infrarust::plugin::{connection_claims, types as wt};
use crate::config::SandboxLimits;
use crate::store_state::build_probe_state;

#[tokio::test]
async fn a_plugin_claims_prefixes_until_its_instance_is_released() {
    let services = PluginServices::for_tests();
    let claims = Arc::clone(&services.connection_claims);
    let factory = PluginContextFactoryImpl::new(
        services,
        HashMap::from([(
            "claimer".to_owned(),
            PluginPermissions {
                permissions: vec![Capability::ConnectionHandoff.to_kebab().to_owned()],
                ..PluginPermissions::default()
            },
        )]),
    );
    let mut state = build_probe_state("claimer".to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::baseline().with(Capability::ConnectionHandoff))
        .with_ctx(factory.create_context("claimer"));
    let s = &mut state;

    connection_claims::Host::register_prefix(s, b"XFER".to_vec())
        .await
        .unwrap()
        .unwrap();
    connection_claims::Host::register_prefix(s, b"XFER".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(s.claim_prefix_count(), 1);

    let invalid = connection_claims::Host::register_prefix(s, vec![0x10, 0x00])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(invalid.kind, wt::ErrorKind::InvalidArgument);
    let unknown = connection_claims::Host::unregister_prefix(s, b"NOPE".to_vec())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(unknown.kind, wt::ErrorKind::NotFound);

    for at in 1..16u8 {
        connection_claims::Host::register_prefix(s, vec![b'B', b'A' + at])
            .await
            .unwrap()
            .unwrap();
    }
    let full = connection_claims::Host::register_prefix(s, b"ZZ".to_vec())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(full.kind, wt::ErrorKind::LimitExceeded);
    assert_eq!(claims.len(), 16);

    connection_claims::Host::unregister_prefix(s, b"XFER".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claims.len(), 15);
    assert_eq!(s.claim_prefix_count(), 15);

    s.release_host_resources();
    assert!(claims.is_empty(), "a released instance keeps no claim");
    assert_eq!(s.claim_prefix_count(), 0);
}
