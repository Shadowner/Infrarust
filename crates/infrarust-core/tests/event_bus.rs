#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Tests for the `EventBus` dispatch engine.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority};

use infrarust_core::event_bus::EventBusImpl;

// --- Test event types ---

struct TestEvent {
    value: i32,
}
impl Event for TestEvent {}

struct OtherEvent {
    flag: bool,
}
impl Event for OtherEvent {}

// --- Tests ---

#[tokio::test]
async fn test_fire_no_handlers() {
    let bus = EventBusImpl::new();
    let event = bus.fire(TestEvent { value: 42 }).await;
    assert_eq!(event.value, 42);
}

#[tokio::test]
async fn test_fire_sync_handler_modifies() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |event| {
        event.value += 10;
    });

    let event = bus.fire(TestEvent { value: 5 }).await;
    assert_eq!(event.value, 15);
}

#[tokio::test]
async fn test_fire_async_handler_modifies() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, |event| -> BoxFuture<'_, ()> {
        Box::pin(async move {
            event.value *= 3;
        })
    });

    let event = bus.fire(TestEvent { value: 7 }).await;
    assert_eq!(event.value, 21);
}

#[tokio::test]
async fn test_fire_and_forget_executes() {
    let bus = Arc::new(EventBusImpl::new());
    let flag = Arc::new(AtomicBool::new(false));
    let flag_clone = Arc::clone(&flag);

    let bus_ref: &dyn EventBus = bus.as_ref();
    bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, move |_event| {
        flag_clone.store(true, Ordering::SeqCst);
    });

    bus.fire_and_forget_arc(TestEvent { value: 0 });

    // Give the spawned task time to run
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_subscribe_returns_unique_handles() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    let h1 = bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |_| {});
    let h2 = bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |_| {});
    let h3 = bus_ref.subscribe::<OtherEvent, _>(EventPriority::NORMAL, |_| {});

    assert_ne!(h1, h2);
    assert_ne!(h2, h3);
    assert_ne!(h1, h3);
}

#[tokio::test]
async fn test_unsubscribe_removes_handler() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    let handle = bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |event| {
        event.value = 999;
    });

    // Handler is active
    let event = bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(event.value, 999);

    // Unsubscribe
    bus_ref.unsubscribe(handle);

    // Handler no longer called
    let event = bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(event.value, 0);
}

#[tokio::test]
async fn test_fire_different_event_types() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |event| {
        event.value = 42;
    });

    // OtherEvent should not trigger TestEvent's handler
    let other = bus.fire(OtherEvent { flag: false }).await;
    assert!(!other.flag);

    // TestEvent should trigger its handler
    let test = bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(test.value, 42);
}

#[tokio::test]
async fn test_fire_mixed_sync_async() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;

    // Sync handler first (EARLY)
    bus_ref.subscribe::<TestEvent, _>(EventPriority::EARLY, |event| {
        event.value += 1;
    });

    // Async handler second (NORMAL)
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, |event| -> BoxFuture<'_, ()> {
        Box::pin(async move {
            event.value *= 10;
        })
    });

    // Sync handler last (LATE)
    bus_ref.subscribe::<TestEvent, _>(EventPriority::LATE, |event| {
        event.value += 5;
    });

    // Execution: 0 + 1 = 1, 1 * 10 = 10, 10 + 5 = 15
    let event = bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(event.value, 15);
}

#[tokio::test]
async fn test_concurrent_fire_and_subscribe() {
    let bus = Arc::new(EventBusImpl::new());

    let bus_ref: &dyn EventBus = bus.as_ref();
    bus_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |event| {
        event.value += 1;
    });

    // Subscribe on a different type from a spawned task during fire
    let bus_for_task = Arc::clone(&bus);
    let handle = tokio::spawn(async move {
        let bus_ref: &dyn EventBus = bus_for_task.as_ref();
        bus_ref.subscribe::<OtherEvent, _>(EventPriority::NORMAL, |event| {
            event.flag = true;
        });
    });

    let event = bus.fire(TestEvent { value: 0 }).await;
    handle.await.unwrap();

    assert_eq!(event.value, 1);

    // Verify the OtherEvent handler was registered
    let other = bus.fire(OtherEvent { flag: false }).await;
    assert!(other.flag);
}
