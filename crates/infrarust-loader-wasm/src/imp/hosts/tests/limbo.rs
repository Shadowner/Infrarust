use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::LimboEntryContext;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::test_util::RecordingLimboSession;
use infrarust_api::types::{GameProfile, PlayerId, ServerId};
use wasmtime::component::{Resource, ResourceTableError};

use crate::bindings::infrarust::plugin::{limbo as wl, types as wt};
use crate::consts::MAX_HOST_HANDLES;

use super::{is_limit_exceeded, quota_state, quotas, state_with};

#[tokio::test]
async fn the_limbo_quota_refuses_a_new_handler_name_but_not_one_already_held() {
    let mut state = quota_state(quotas(|q| q.limbo_handlers = 1), &[Capability::Limbo]);
    assert_eq!(
        wl::Host::register_limbo_handler(&mut state, "gate".into(), 1)
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        wl::Host::register_limbo_handler(&mut state, "gate".into(), 2)
            .await
            .unwrap(),
        Ok(())
    );
    let over = wl::Host::register_limbo_handler(&mut state, "other".into(), 3)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "limbo_handlers"), "{over:?}");
}

struct NativeGate;

impl infrarust_api::limbo::LimboHandler for NativeGate {
    fn name(&self) -> &str {
        "taken"
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn infrarust_api::limbo::LimboSession,
    ) -> BoxFuture<'a, infrarust_api::limbo::HandlerResult> {
        Box::pin(async { infrarust_api::limbo::HandlerResult::Accept })
    }
}

#[tokio::test]
async fn a_refused_limbo_handler_takes_no_room_and_is_refused_again() {
    let mut owner = quota_state(quotas(|q| q.limbo_handlers = 1), &[Capability::Limbo]);
    let shared_ctx = owner.services().unwrap();
    shared_ctx
        .register_limbo_handler(Box::new(NativeGate))
        .unwrap();
    let first = wl::Host::register_limbo_handler(&mut owner, "taken".into(), 1)
        .await
        .unwrap();
    assert!(
        matches!(&first, Err(e) if e.kind == wt::ErrorKind::Conflict),
        "{first:?}"
    );
    let again = wl::Host::register_limbo_handler(&mut owner, "taken".into(), 2)
        .await
        .unwrap();
    assert!(
        matches!(&again, Err(e) if e.kind == wt::ErrorKind::Conflict),
        "{again:?}"
    );
    assert_eq!(
        wl::Host::register_limbo_handler(&mut owner, "free".into(), 3)
            .await
            .unwrap(),
        Ok(())
    );
}

#[tokio::test]
async fn acquiring_handles_in_a_loop_is_refused_at_the_host_handle_cap() {
    let mut state = state_with(CapabilitySet::default(), vec![]);
    let session = RecordingLimboSession::new(
        PlayerId::new(1),
        GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "tester".to_owned(),
            properties: vec![],
        },
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    );
    let session = state.push_limbo_session(session).unwrap();

    let mut held = Vec::new();
    let refused = loop {
        let borrowed = Resource::new_borrow(session.rep());
        match wl::HostLimboSession::acquire_handle(&mut state, borrowed).await {
            Ok(handle) => held.push(handle),
            Err(error) => break error,
        }
        assert!(
            held.len() < MAX_HOST_HANDLES,
            "the handle cap was never reached"
        );
    };
    assert_eq!(held.len(), MAX_HOST_HANDLES - 1);
    assert!(
        matches!(
            refused.downcast_ref::<ResourceTableError>(),
            Some(ResourceTableError::Full)
        ),
        "{refused:?}"
    );

    let released = held.pop().unwrap();
    wl::HostLimboSessionHandle::drop(&mut state, released)
        .await
        .unwrap();
    let borrowed = Resource::new_borrow(session.rep());
    assert!(
        wl::HostLimboSession::acquire_handle(&mut state, borrowed)
            .await
            .is_ok()
    );
}
