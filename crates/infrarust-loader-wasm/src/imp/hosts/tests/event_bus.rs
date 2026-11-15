use std::sync::{Arc, Mutex};

use infrarust_api::event::EventPriority;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::permissions::{Capability, CapabilitySet};

use crate::bindings::infrarust::plugin::events::EventKind;
use crate::bindings::infrarust::plugin::{event_bus, types as wt};

use super::{is_limit_exceeded, quota_state, quotas, state_with};

pub(super) fn packet_filter() -> event_bus::PacketFilter {
    event_bus::PacketFilter {
        packet_id: 3,
        state: wt::ConnectionState::Play,
        direction: wt::PacketDirection::Serverbound,
    }
}

#[tokio::test]
async fn fire_named_returns_what_the_native_listeners_decided() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    let ctx = state.services().unwrap();
    let seen_source = Arc::new(Mutex::new(String::new()));
    let source = Arc::clone(&seen_source);
    ctx.event_bus()
        .subscribe::<NamedEvent, _>(EventPriority::NORMAL, move |event| {
            if event.name == "echo" {
                *source.lock().unwrap() = event.source_plugin.clone();
                event.respond("text/plain", event.payload.clone());
                event.cancel();
            }
        });

    let answered = event_bus::Host::fire_named(
        &mut state,
        "echo".into(),
        "text/plain".into(),
        b"hi".to_vec(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(answered.cancelled);
    let response = answered.response.expect("a response");
    assert_eq!(response.payload, b"hi");
    assert_eq!(*seen_source.lock().unwrap(), "test");

    let ignored = event_bus::Host::fire_named(&mut state, "other".into(), "x".into(), vec![])
        .await
        .unwrap()
        .unwrap();
    assert!(!ignored.cancelled);
    assert_eq!(ignored.response, None);
}

#[tokio::test]
async fn subscribe_packets_needs_a_filter_and_registers_one_native_listener_per_filter() {
    let mut state = state_with(
        CapabilitySet::baseline().with(Capability::RawPacket),
        vec![],
    );
    let empty = event_bus::Host::subscribe_packets(&mut state, vec![], 128)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(empty.kind, wt::ErrorKind::InvalidArgument);
    let listener = event_bus::Host::subscribe_packets(&mut state, vec![packet_filter()], 128)
        .await
        .unwrap()
        .unwrap();
    let ctx = state.services().unwrap();
    assert!(ctx.event_bus().has_packet_listeners(
        3,
        infrarust_api::event::ConnectionState::Play,
        infrarust_api::events::packet::PacketDirection::Serverbound
    ));
    assert_eq!(
        event_bus::Host::unsubscribe(&mut state, listener)
            .await
            .unwrap(),
        Ok(true)
    );
    assert!(!ctx.event_bus().has_packet_listeners(
        3,
        infrarust_api::event::ConnectionState::Play,
        infrarust_api::events::packet::PacketDirection::Serverbound
    ));
    let refused = event_bus::Host::subscribe(&mut state, EventKind::RawPacket, 128)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn the_listener_quota_counts_every_native_listener_and_unsubscribing_frees_room() {
    let mut state = quota_state(quotas(|q| q.event_listeners = 3), &[Capability::RawPacket]);
    let first = event_bus::Host::subscribe(&mut state, EventKind::PostLogin, 0)
        .await
        .unwrap()
        .unwrap();
    let two_filters = vec![packet_filter(), packet_filter()];
    event_bus::Host::subscribe_packets(&mut state, two_filters.clone(), 0)
        .await
        .unwrap()
        .unwrap();
    let over = event_bus::Host::subscribe_packets(&mut state, two_filters, 0)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "event_listeners"), "{over:?}");
    let named = event_bus::Host::subscribe_named(&mut state, "full".into(), 0)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&named, "event_listeners"), "{named:?}");

    assert_eq!(
        event_bus::Host::unsubscribe(&mut state, first)
            .await
            .unwrap(),
        Ok(true)
    );
    assert!(
        event_bus::Host::subscribe_named(&mut state, "room".into(), 0)
            .await
            .unwrap()
            .is_ok()
    );
}
