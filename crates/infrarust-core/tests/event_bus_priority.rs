//! Tests for EventBus handler priority ordering.

use std::sync::{Arc, Mutex};

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{Event, EventPriority};

use infrarust_core::event_bus::EventBusImpl;

struct TraceEvent {
    value: i32,
    order: Vec<&'static str>,
}
impl Event for TraceEvent {}

#[tokio::test]
async fn test_priority_order_first_last() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    // Register LAST first, then FIRST — dispatch should still be FIRST then LAST
    let order = Arc::new(Mutex::new(Vec::<&str>::new()));

    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::LAST, move |_| {
        o.lock().unwrap().push("LAST");
    });

    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::FIRST, move |_| {
        o.lock().unwrap().push("FIRST");
    });

    bus.fire(TraceEvent {
        value: 0,
        order: vec![],
    })
    .await;

    let result = order.lock().unwrap();
    assert_eq!(*result, vec!["FIRST", "LAST"]);
}

#[tokio::test]
async fn test_priority_order_sequential() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    let order = Arc::new(Mutex::new(Vec::<&str>::new()));

    // Register in scrambled order
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::LAST, move |_| {
        o.lock().unwrap().push("LAST");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::FIRST, move |_| {
        o.lock().unwrap().push("FIRST");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::NORMAL, move |_| {
        o.lock().unwrap().push("NORMAL");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::EARLY, move |_| {
        o.lock().unwrap().push("EARLY");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::LATE, move |_| {
        o.lock().unwrap().push("LATE");
    });

    bus.fire(TraceEvent {
        value: 0,
        order: vec![],
    })
    .await;

    let result = order.lock().unwrap();
    assert_eq!(*result, vec!["FIRST", "EARLY", "NORMAL", "LATE", "LAST"]);
}

#[tokio::test]
async fn test_handler_sees_previous_changes() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    // FIRST sets value to 10
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::FIRST, |event| {
        event.value = 10;
    });

    // NORMAL sees value = 10 and doubles it
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::NORMAL, |event| {
        event.value *= 2;
    });

    let event = bus
        .fire(TraceEvent {
            value: 0,
            order: vec![],
        })
        .await;
    assert_eq!(event.value, 20);
}

#[tokio::test]
async fn test_same_priority_insertion_order() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    let order = Arc::new(Mutex::new(Vec::<&str>::new()));

    // All NORMAL — should execute in insertion order
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::NORMAL, move |_| {
        o.lock().unwrap().push("first_registered");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::NORMAL, move |_| {
        o.lock().unwrap().push("second_registered");
    });
    let o = Arc::clone(&order);
    bus_ref.subscribe::<TraceEvent, _>(EventPriority::NORMAL, move |_| {
        o.lock().unwrap().push("third_registered");
    });

    bus.fire(TraceEvent {
        value: 0,
        order: vec![],
    })
    .await;

    let result = order.lock().unwrap();
    assert_eq!(
        *result,
        vec!["first_registered", "second_registered", "third_registered"]
    );
}
