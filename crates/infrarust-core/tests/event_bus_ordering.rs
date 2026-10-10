#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{Event, EventPriority};
use infrarust_core::event_bus::{CORE_OWNER, DiagnosticKind, EventBusConfig, EventBusImpl};
use infrarust_core::plugin::tracking::TrackingEventBus;
use tokio::time::Instant;

struct Numbered(u32);
impl Event for Numbered {}

struct Unrelated;
impl Event for Unrelated {}

fn config(handler_timeout: Duration) -> EventBusConfig {
    EventBusConfig {
        handler_timeout,
        slow_handler_threshold: handler_timeout,
        packet_handler_timeout: handler_timeout,
    }
}

fn started_bus() -> Arc<EventBusImpl> {
    let bus = Arc::new(EventBusImpl::new());
    bus.start_dispatcher();
    bus
}

fn record_sync(bus: &dyn EventBus) -> Arc<Mutex<Vec<u32>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    bus.subscribe::<Numbered, _>(EventPriority::NORMAL, move |event| {
        sink.lock().unwrap().push(event.0);
    });
    seen
}

fn record_async(bus: &dyn EventBus) -> Arc<Mutex<Vec<u32>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    bus.subscribe_async::<Numbered, _>(EventPriority::NORMAL, move |event| {
        let sink = Arc::clone(&sink);
        let value = event.0;
        Box::pin(async move {
            tokio::task::yield_now().await;
            sink.lock().unwrap().push(value);
        })
    });
    seen
}

async fn let_spawned_tasks_run() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thousand_posted_events_are_observed_in_order() {
    let bus = started_bus();
    let seen = record_async(bus.as_ref());

    for n in 0..1000 {
        bus.post(Numbered(n));
    }
    bus.flush().await;

    assert_eq!(*seen.lock().unwrap(), (0..1000).collect::<Vec<_>>());
}

#[tokio::test]
async fn events_posted_before_the_dispatcher_starts_are_buffered() {
    let bus = Arc::new(EventBusImpl::new());
    let seen = record_sync(bus.as_ref());

    bus.post(Numbered(1));
    bus.post(Numbered(2));
    bus.post(Numbered(3));
    let_spawned_tasks_run().await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "{:?}",
        seen.lock().unwrap()
    );

    bus.start_dispatcher();
    bus.flush().await;

    assert_eq!(*seen.lock().unwrap(), [1, 2, 3]);
}

#[tokio::test]
async fn starting_the_dispatcher_twice_delivers_each_event_once() {
    let bus = Arc::new(EventBusImpl::new());
    let seen = record_async(bus.as_ref());

    bus.post(Numbered(1));
    bus.start_dispatcher();
    bus.start_dispatcher();
    bus.post(Numbered(2));
    bus.flush().await;

    assert_eq!(*seen.lock().unwrap(), [1, 2]);
}

#[tokio::test(start_paused = true)]
async fn flush_waits_for_a_slow_async_handler() {
    let bus = Arc::new(EventBusImpl::with_config(config(Duration::from_secs(60))));
    bus.start_dispatcher();
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&done);
    let core_ref: &dyn EventBus = bus.as_ref();
    core_ref.subscribe_async::<Numbered, _>(EventPriority::NORMAL, move |_| {
        let finished = Arc::clone(&finished);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            finished.store(true, Ordering::SeqCst);
        })
    });
    let started = Instant::now();

    bus.post(Numbered(0));
    bus.flush().await;

    assert!(done.load(Ordering::SeqCst));
    assert!(started.elapsed() >= Duration::from_secs(5));
}

#[tokio::test]
async fn flush_with_nothing_posted_returns() {
    let bus = started_bus();

    bus.flush().await;
}

#[tokio::test]
async fn a_panicking_handler_does_not_stop_later_posted_events() {
    let bus = started_bus();
    let mut diagnostics = bus.diagnostics();
    let plugin = TrackingEventBus::new(Arc::clone(&bus), "panicky");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe::<Numbered, _>(EventPriority::FIRST, |event| {
        if event.0 == 1 {
            panic!("refusing event one");
        }
    });
    let seen = record_sync(bus.as_ref());

    bus.post(Numbered(0));
    bus.post(Numbered(1));
    bus.post(Numbered(2));
    bus.flush().await;

    assert_eq!(*seen.lock().unwrap(), [0, 1, 2]);
    let diagnostic = diagnostics.try_recv().unwrap();
    assert_eq!(&*diagnostic.owner, "panicky");
    assert_eq!(&*diagnostic.fired_by, CORE_OWNER);
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Panicked {
            message: "refusing event one".to_string()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn a_hung_handler_holds_the_queue_for_the_timeout_only() {
    let bus = Arc::new(EventBusImpl::with_config(config(Duration::from_secs(1))));
    bus.start_dispatcher();
    let mut diagnostics = bus.diagnostics();
    let plugin = TrackingEventBus::new(Arc::clone(&bus), "stuck");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe_async::<Numbered, _>(EventPriority::FIRST, |event| {
        if event.0 == 0 {
            Box::pin(std::future::pending())
        } else {
            Box::pin(async {})
        }
    });
    let seen = record_sync(bus.as_ref());
    let started = Instant::now();

    bus.post(Numbered(0));
    bus.post(Numbered(1));
    bus.flush().await;

    assert_eq!(*seen.lock().unwrap(), [0, 1]);
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(2));
    let diagnostic = diagnostics.try_recv().unwrap();
    assert_eq!(&*diagnostic.owner, "stuck");
    assert_eq!(diagnostic.kind, DiagnosticKind::TimedOut);
}

#[tokio::test]
async fn the_dispatcher_does_not_keep_the_bus_alive() {
    let bus = started_bus();
    let weak = Arc::downgrade(&bus);

    drop(bus);
    let_spawned_tasks_run().await;

    assert!(weak.upgrade().is_none());
}

#[test]
fn has_listeners_follows_subscribe_and_unsubscribe() {
    let bus = Arc::new(EventBusImpl::new());
    assert!(!bus.has_listeners::<Numbered>());

    let core_ref: &dyn EventBus = bus.as_ref();
    let core = core_ref.subscribe::<Numbered, _>(EventPriority::NORMAL, |_| {});
    assert!(bus.has_listeners::<Numbered>());
    assert!(!bus.has_listeners::<Unrelated>());

    let plugin = TrackingEventBus::new(Arc::clone(&bus), "plugin");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe::<Unrelated, _>(EventPriority::NORMAL, |_| {});
    assert!(bus.has_listeners::<Unrelated>());

    assert!(bus.unsubscribe(core));
    assert!(!bus.has_listeners::<Numbered>());

    plugin.unsubscribe_all();
    assert!(!bus.has_listeners::<Unrelated>());
}
