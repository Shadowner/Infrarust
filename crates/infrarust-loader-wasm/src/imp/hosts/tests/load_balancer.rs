use infrarust_api::permissions::CapabilitySet;

use crate::bindings::infrarust::plugin::{config_service, load_balancer, types as wt};

use super::{address, state_with};

#[tokio::test]
async fn load_balancer_reads_and_config_writes_reach_the_native_services() {
    let mut state = state_with(CapabilitySet::native_trusted(), vec![]);
    let backends = load_balancer::Host::backends(&mut state, "lobby".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backends.len(), 1);
    assert_eq!(backends[0].address, address());
    assert_eq!(
        load_balancer::Host::set_drained(&mut state, "lobby".into(), address(), true)
            .await
            .unwrap(),
        Ok(())
    );
    let refused = config_service::Host::write_proxy_config_document(&mut state, "bind = 1".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::PermissionDenied);
    assert_eq!(
        config_service::Host::list_server_sources(&mut state)
            .await
            .unwrap(),
        Ok(vec![])
    );
}
