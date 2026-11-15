use infrarust_api::permissions::{Capability, CapabilitySet};

use crate::bindings::infrarust::plugin::{plugin_registry, proxy_info, types as wt};

use super::state_with;

#[tokio::test]
async fn proxy_info_and_the_plugin_registry_answer_without_any_capability() {
    let granted = CapabilitySet::default()
        .with(Capability::Ban)
        .with(Capability::PluginMessaging);
    let mut state = state_with(granted, vec![]);
    let mut capabilities = proxy_info::Host::granted_capabilities(&mut state)
        .await
        .unwrap();
    capabilities.sort_by_key(|c| *c as u8);
    assert_eq!(
        capabilities,
        [wt::Capability::Ban, wt::Capability::PluginMessaging]
    );
    let details = proxy_info::Host::details(&mut state).await.unwrap();
    assert_eq!(details.bind.port, 25565);
    assert_eq!(
        details.unknown_domain_behavior,
        proxy_info::UnknownDomainBehavior::DefaultMotd
    );
    assert!(
        plugin_registry::Host::list(&mut state)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        plugin_registry::Host::get(&mut state, "nobody".into())
            .await
            .unwrap(),
        None
    );
}
