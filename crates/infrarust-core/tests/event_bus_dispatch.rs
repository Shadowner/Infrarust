#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{
    BoxFuture, ConnectionState, Event, EventPriority, ListenerHandle, PacketDirection,
    PacketFilter, ResultedEvent,
};
use infrarust_api::events::packet::{RawPacketEvent, RawPacketResult};
use infrarust_api::types::{PlayerId, RawPacket};
use infrarust_core::event_bus::{
    CORE_OWNER, DiagnosticKind, EventBusConfig, EventBusImpl, HandlerDiagnostic,
};
use infrarust_core::plugin::tracking::TrackingEventBus;
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time::Instant;

struct TestEvent {
    value: i32,
}
impl Event for TestEvent {}

fn config(handler_timeout: Duration, slow_handler_threshold: Duration) -> EventBusConfig {
    EventBusConfig {
        handler_timeout,
        slow_handler_threshold,
        packet_handler_timeout: handler_timeout,
    }
}

fn plugin_bus(bus: &Arc<EventBusImpl>, plugin_id: &str) -> TrackingEventBus {
    TrackingEventBus::new(Arc::clone(bus), plugin_id)
}

fn drain(rx: &mut Receiver<HandlerDiagnostic>) -> Vec<HandlerDiagnostic> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(diagnostic) => out.push(diagnostic),
            Err(TryRecvError::Empty | TryRecvError::Closed) => return out,
            Err(TryRecvError::Lagged(_)) => {}
        }
    }
}

fn counter(bus: &dyn EventBus, priority: EventPriority) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    bus.subscribe::<TestEvent, _>(priority, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    count
}

fn panicked(diagnostic: &HandlerDiagnostic) -> &str {
    match &diagnostic.kind {
        DiagnosticKind::Panicked { message } => message,
        other => panic!("expected a panic diagnostic, got {other:?}"),
    }
}

#[tokio::test]
async fn sync_panic_is_isolated_and_the_next_handler_runs() {
    let bus = Arc::new(EventBusImpl::new());
    let mut diagnostics = bus.diagnostics();
    let plugin = plugin_bus(&bus, "panicky");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe::<TestEvent, _>(EventPriority::FIRST, |_| panic!("sync boom"));
    let after = counter(bus.as_ref(), EventPriority::LAST);

    let event = bus.fire(TestEvent { value: 1 }).await;

    assert_eq!(event.value, 1);
    assert_eq!(after.load(Ordering::SeqCst), 1);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "panicky");
    assert_eq!(diagnostics[0].event, "TestEvent");
    assert_eq!(panicked(&diagnostics[0]), "sync boom");
}

#[tokio::test]
async fn async_panic_while_building_the_future_is_isolated() {
    let bus = Arc::new(EventBusImpl::new());
    let mut diagnostics = bus.diagnostics();
    let plugin = plugin_bus(&bus, "eager");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe_async::<TestEvent, _>(EventPriority::FIRST, |_| -> BoxFuture<'_, ()> {
        panic!("built badly")
    });
    let after = counter(bus.as_ref(), EventPriority::LAST);

    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(after.load(Ordering::SeqCst), 1);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "eager");
    assert_eq!(panicked(&diagnostics[0]), "built badly");
}

#[tokio::test]
async fn async_panic_after_an_await_is_isolated() {
    let bus = Arc::new(EventBusImpl::new());
    let mut diagnostics = bus.diagnostics();
    let plugin = plugin_bus(&bus, "late");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe_async::<TestEvent, _>(EventPriority::FIRST, |_| {
        Box::pin(async {
            tokio::task::yield_now().await;
            panic!("{}", String::from("after await"));
        })
    });
    let after = counter(bus.as_ref(), EventPriority::LAST);

    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(after.load(Ordering::SeqCst), 1);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "late");
    assert_eq!(panicked(&diagnostics[0]), "after await");
}

#[tokio::test]
async fn a_panicking_handler_keeps_the_changes_it_made_first() {
    let bus = EventBusImpl::new();
    let bus_ref: &dyn EventBus = &bus;
    bus_ref.subscribe::<TestEvent, _>(EventPriority::FIRST, |event| {
        event.value += 10;
        panic!("after sync write");
    });
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, |event| {
        Box::pin(async move {
            event.value += 100;
            tokio::task::yield_now().await;
            panic!("after async write");
        })
    });

    let event = bus.fire(TestEvent { value: 1 }).await;

    assert_eq!(event.value, 111);
}

#[tokio::test(start_paused = true)]
async fn a_hung_async_handler_is_cancelled_at_the_timeout() {
    let bus = Arc::new(EventBusImpl::with_config(config(
        Duration::from_millis(200),
        Duration::from_secs(1),
    )));
    let mut diagnostics = bus.diagnostics();
    let dropped = Arc::new(AtomicBool::new(false));
    let plugin = plugin_bus(&bus, "stuck");
    let plugin_ref: &dyn EventBus = &plugin;
    let flag = Arc::clone(&dropped);
    plugin_ref.subscribe_async::<TestEvent, _>(EventPriority::FIRST, move |event| {
        let guard = DropFlag(Arc::clone(&flag));
        Box::pin(async move {
            let _guard = guard;
            event.value = 7;
            std::future::pending::<()>().await;
        })
    });
    let after = counter(bus.as_ref(), EventPriority::LAST);

    let started = Instant::now();
    let event = tokio::time::timeout(Duration::from_secs(30), bus.fire(TestEvent { value: 0 }))
        .await
        .expect("fire must return once the handler times out");
    let elapsed = started.elapsed();

    assert_eq!(elapsed, Duration::from_millis(200));
    assert_eq!(event.value, 7);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the hung future was not dropped"
    );
    assert_eq!(after.load(Ordering::SeqCst), 1);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "stuck");
    assert_eq!(diagnostics[0].kind, DiagnosticKind::TimedOut);
    assert_eq!(diagnostics[0].elapsed, Duration::from_millis(200));
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn a_slow_async_handler_is_reported_and_completes() {
    let bus = Arc::new(EventBusImpl::with_config(config(
        Duration::from_secs(10),
        Duration::from_secs(1),
    )));
    let mut diagnostics = bus.diagnostics();
    let plugin = plugin_bus(&bus, "sluggish");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, |event| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            event.value = 42;
        })
    });

    let event = bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(event.value, 42);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "sluggish");
    assert_eq!(diagnostics[0].kind, DiagnosticKind::Slow);
    assert_eq!(diagnostics[0].elapsed, Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn a_fast_handler_produces_no_diagnostic() {
    let bus = Arc::new(EventBusImpl::with_config(config(
        Duration::from_secs(10),
        Duration::from_secs(1),
    )));
    let mut diagnostics = bus.diagnostics();
    let bus_ref: &dyn EventBus = bus.as_ref();
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, |event| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            event.value = 1;
        })
    });

    bus.fire(TestEvent { value: 0 }).await;

    assert!(drain(&mut diagnostics).is_empty());
}

#[tokio::test]
async fn a_sync_handler_overrunning_the_timeout_is_reported_as_slow() {
    let bus = Arc::new(EventBusImpl::with_config(config(
        Duration::from_millis(20),
        Duration::from_millis(10),
    )));
    let mut diagnostics = bus.diagnostics();
    let plugin = plugin_bus(&bus, "blocking");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |event| {
        std::thread::sleep(Duration::from_millis(40));
        event.value = 3;
    });

    let event = bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(event.value, 3);
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "blocking");
    assert_eq!(diagnostics[0].kind, DiagnosticKind::Slow);
    assert!(diagnostics[0].elapsed >= Duration::from_millis(40));
}

#[tokio::test(start_paused = true)]
async fn priority_order_holds_when_handlers_fail() {
    let bus = EventBusImpl::with_config(config(Duration::from_millis(100), Duration::from_secs(1)));
    let bus_ref: &dyn EventBus = &bus;
    let order = Arc::new(Mutex::new(Vec::new()));

    let log = Arc::clone(&order);
    bus_ref.subscribe::<TestEvent, _>(EventPriority::LAST, move |_| {
        log.lock().unwrap().push("last");
    });
    let log = Arc::clone(&order);
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::LATE, move |_| {
        log.lock().unwrap().push("late-hangs");
        Box::pin(std::future::pending())
    });
    let log = Arc::clone(&order);
    bus_ref.subscribe::<TestEvent, _>(EventPriority::FIRST, move |_| {
        log.lock().unwrap().push("first-panics");
        panic!("first");
    });
    let log = Arc::clone(&order);
    bus_ref.subscribe_async::<TestEvent, _>(EventPriority::NORMAL, move |_| {
        let log = Arc::clone(&log);
        Box::pin(async move {
            log.lock().unwrap().push("normal-panics");
            panic!("normal");
        })
    });
    let log = Arc::clone(&order);
    bus_ref.subscribe::<TestEvent, _>(EventPriority::EARLY, move |_| {
        log.lock().unwrap().push("early");
    });

    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(
        *order.lock().unwrap(),
        [
            "first-panics",
            "early",
            "normal-panics",
            "late-hangs",
            "last"
        ]
    );
}

#[tokio::test]
async fn unsubscribing_during_dispatch_skips_a_later_handler() {
    let bus = Arc::new(EventBusImpl::new());
    let target: Arc<OnceLock<ListenerHandle>> = Arc::new(OnceLock::new());
    let weak: Weak<EventBusImpl> = Arc::downgrade(&bus);
    let slot = Arc::clone(&target);
    let bus_ref: &dyn EventBus = bus.as_ref();
    bus_ref.subscribe::<TestEvent, _>(EventPriority::FIRST, move |_| {
        if let (Some(bus), Some(handle)) = (weak.upgrade(), slot.get()) {
            assert!(bus.unsubscribe(*handle));
        }
    });
    let called = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&called);
    let handle = bus_ref.subscribe::<TestEvent, _>(EventPriority::LAST, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    target.set(handle).unwrap();

    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(called.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn subscribing_during_dispatch_waits_for_the_next_event() {
    let bus = Arc::new(EventBusImpl::new());
    let weak = Arc::downgrade(&bus);
    let late_calls = Arc::new(AtomicUsize::new(0));
    let registered = Arc::new(AtomicBool::new(false));
    let calls = Arc::clone(&late_calls);
    let once = Arc::clone(&registered);
    let bus_ref: &dyn EventBus = bus.as_ref();
    bus_ref.subscribe::<TestEvent, _>(EventPriority::FIRST, move |_| {
        if once.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(bus) = weak.upgrade() {
            let calls = Arc::clone(&calls);
            let bus_ref: &dyn EventBus = bus.as_ref();
            bus_ref.subscribe::<TestEvent, _>(EventPriority::LAST, move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
            });
        }
    });

    bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(late_calls.load(Ordering::SeqCst), 0);

    bus.fire(TestEvent { value: 0 }).await;
    assert_eq!(late_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn the_tracking_bus_subscribes_under_the_plugin_id() {
    let bus = Arc::new(EventBusImpl::new());
    let plugin = plugin_bus(&bus, "owner-a");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, |_| {});
    let core_ref: &dyn EventBus = bus.as_ref();
    core_ref.subscribe::<TestEvent, _>(EventPriority::LAST, |_| {});

    let owners: Vec<String> = bus
        .listener_owners::<TestEvent>()
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(owners, ["owner-a", CORE_OWNER]);
}

#[tokio::test]
async fn a_plugin_cannot_unsubscribe_another_plugins_listener() {
    let bus = Arc::new(EventBusImpl::new());
    let plugin_a = plugin_bus(&bus, "plugin-a");
    let plugin_b = plugin_bus(&bus, "plugin-b");
    let a_ref: &dyn EventBus = &plugin_a;
    let b_ref: &dyn EventBus = &plugin_b;
    let b_calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&b_calls);
    let b_handle = b_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });

    assert!(!a_ref.unsubscribe(b_handle));
    assert!(!a_ref.unsubscribe(ListenerHandle::from_raw(b_handle.as_u64())));
    assert!(!bus.unsubscribe(b_handle));
    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(b_calls.load(Ordering::SeqCst), 1);
    assert_eq!(plugin_b.tracked_count(), 1);
}

#[tokio::test]
async fn unsubscribing_untracks_the_handle() {
    let bus = Arc::new(EventBusImpl::new());
    let plugin = plugin_bus(&bus, "tidy");
    let plugin_ref: &dyn EventBus = &plugin;
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let handle = plugin_ref.subscribe::<TestEvent, _>(EventPriority::NORMAL, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(plugin.tracked_count(), 1);

    assert!(plugin_ref.unsubscribe(handle));
    assert_eq!(plugin.tracked_count(), 0);
    assert!(!plugin_ref.unsubscribe(handle));
    bus.fire(TestEvent { value: 0 }).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_panicking_packet_handler_lets_the_packet_pass() {
    let bus = Arc::new(EventBusImpl::new());
    let mut diagnostics = bus.diagnostics();
    let filter = PacketFilter {
        packet_id: 0x1A,
        state: ConnectionState::Play,
        direction: PacketDirection::Serverbound,
    };
    let plugin = plugin_bus(&bus, "packet-plugin");
    let plugin_ref: &dyn EventBus = &plugin;
    plugin_ref.subscribe_packet_typed(filter, EventPriority::FIRST, |_| panic!("bad packet"));
    let later = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&later);
    let core_ref: &dyn EventBus = bus.as_ref();
    core_ref.subscribe_packet_typed(filter, EventPriority::LAST, move |_| {
        seen.store(true, Ordering::SeqCst);
    });

    let mut event = RawPacketEvent::new(
        PlayerId::new(1),
        PacketDirection::Serverbound,
        RawPacket::new(0x1A, bytes::Bytes::from_static(b"payload")),
    );
    bus.fire_packet_event(
        0x1A,
        ConnectionState::Play,
        PacketDirection::Serverbound,
        &mut event,
    )
    .await;

    assert!(matches!(event.result(), RawPacketResult::Pass));
    assert!(later.load(Ordering::SeqCst));
    let diagnostics = drain(&mut diagnostics);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(&*diagnostics[0].owner, "packet-plugin");
    assert_eq!(diagnostics[0].event, "RawPacketEvent");
    assert_eq!(panicked(&diagnostics[0]), "bad packet");
}
