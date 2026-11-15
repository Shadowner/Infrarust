//! `EventBusImpl` — the dispatch engine for Infrarust events.
//!
//! Uses a snapshot pattern: handlers are stored in `Arc<Vec<HandlerEntry>>`.
//! Dispatch clones the `Arc` and iterates the snapshot, so async handlers
//! never hold a lock across `.await` points.

use std::any::{Any, TypeId, type_name};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use futures_util::FutureExt;
use infrarust_api::event::bus::{
    ErasedAsyncHandler, ErasedEvent, ErasedHandler, EventBus, FireError,
};
use infrarust_api::event::{
    BoxFuture, ConnectionState, Event, EventPriority, ListenerHandle, PacketDirection,
    PacketFilter, ResultedEvent,
};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_config::EventsConfig;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;

use queue::Queued;
use registry::{Handlers, LockedRegistry};

use super::builtin::is_builtin_event;
use super::diagnostic::{DiagnosticKind, HandlerDiagnostic, short_type_name};
use super::handler::{HandlerEntry, HandlerKind};
use crate::util::guard::panic_message;

mod queue;
mod registry;

pub const CORE_OWNER: &str = "infrarust";

const DIAGNOSTIC_CAPACITY: usize = 256;
pub const POSTED_EVENT_CAPACITY: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventBusConfig {
    pub handler_timeout: Duration,
    pub slow_handler_threshold: Duration,
    pub packet_handler_timeout: Duration,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self::from(&EventsConfig::default())
    }
}

impl From<&EventsConfig> for EventBusConfig {
    fn from(config: &EventsConfig) -> Self {
        Self {
            handler_timeout: config.handler_timeout,
            slow_handler_threshold: config.slow_handler_threshold,
            packet_handler_timeout: config.packet_handler_timeout,
        }
    }
}

#[derive(Hash, Eq, PartialEq, Clone, Copy)]
struct PacketKey {
    packet_id: i32,
    state: ConnectionState,
    direction: PacketDirection,
}

pub struct EventBusImpl {
    handlers: LockedRegistry<TypeId>,

    packet_handlers: LockedRegistry<PacketKey>,

    next_handle: AtomicU64,
    packet_listener_count: AtomicU64,
    config: EventBusConfig,
    diagnostics: broadcast::Sender<HandlerDiagnostic>,
    core_owner: Arc<str>,
    queue: mpsc::Sender<Queued>,
    undispatched: Mutex<Option<mpsc::Receiver<Queued>>>,
    dropped_posts: AtomicU64,
}

impl EventBusImpl {
    pub fn new() -> Self {
        Self::with_config(EventBusConfig::default())
    }

    pub fn with_config(config: EventBusConfig) -> Self {
        let (queue, undispatched) = mpsc::channel(POSTED_EVENT_CAPACITY);
        Self {
            handlers: LockedRegistry::new(),
            packet_handlers: LockedRegistry::new(),
            next_handle: AtomicU64::new(1),
            packet_listener_count: AtomicU64::new(0),
            config,
            diagnostics: broadcast::channel(DIAGNOSTIC_CAPACITY).0,
            core_owner: Arc::from(CORE_OWNER),
            queue,
            undispatched: Mutex::new(Some(undispatched)),
            dropped_posts: AtomicU64::new(0),
        }
    }

    pub const fn config(&self) -> EventBusConfig {
        self.config
    }

    pub fn diagnostics(&self) -> broadcast::Receiver<HandlerDiagnostic> {
        self.diagnostics.subscribe()
    }

    pub fn listener_owners<E: Event>(&self) -> Vec<Arc<str>> {
        self.handlers
            .snapshot(&TypeId::of::<E>())
            .map(|entries| entries.iter().map(|e| Arc::clone(&e.owner)).collect())
            .unwrap_or_default()
    }

    /// Dispatches an event and awaits all handlers sequentially.
    ///
    /// Handlers are invoked in priority order. The event is returned
    /// after all handlers have executed, potentially modified.
    ///
    /// Used for events whose result matters to the caller (e.g.
    /// `ProxyPingEvent`, `ProxyInitializeEvent`).
    pub async fn fire<E: Event>(&self, mut event: E) -> E {
        self.dispatch(
            TypeId::of::<E>(),
            type_name::<E>(),
            &mut event,
            &self.core_owner,
        )
        .await;
        event
    }

    pub async fn fire_decided<E>(&self, mut event: E) -> (E, Option<Arc<str>>)
    where
        E: ResultedEvent,
        E::Result: Clone + PartialEq,
    {
        let handlers = self.handlers.snapshot(&TypeId::of::<E>());
        let mut decided_by = None;
        let mut last = event.result().clone();
        self.run_handlers(
            handlers,
            &mut event,
            type_name::<E>(),
            &self.core_owner,
            self.config.handler_timeout,
            |entry, event| {
                let Some(event) = event.downcast_ref::<E>() else {
                    return;
                };
                if *event.result() != last {
                    decided_by = Some(Arc::clone(&entry.owner));
                    last = event.result().clone();
                }
            },
        )
        .await;
        (event, decided_by)
    }

    pub fn has_listeners<E: Event>(&self) -> bool {
        self.handlers.has(&TypeId::of::<E>())
    }

    pub(crate) async fn fire_from(
        &self,
        fired_by: &Arc<str>,
        event: &mut dyn ErasedEvent,
    ) -> Result<(), FireError> {
        let event_type = event.type_name();
        let event = event.as_any_mut();
        let type_id = (*event).type_id();
        if is_builtin_event(type_id) {
            tracing::warn!(
                plugin = %fired_by,
                event = short_type_name(event_type),
                "refused to let a plugin fire a built-in proxy event"
            );
            return Err(FireError::Reserved);
        }
        self.dispatch(type_id, event_type, event, fired_by).await;
        Ok(())
    }

    async fn dispatch(
        &self,
        type_id: TypeId,
        event_type: &'static str,
        event: &mut (dyn Any + Send),
        fired_by: &Arc<str>,
    ) {
        let handlers = self.handlers.snapshot(&type_id);
        self.run_handlers(
            handlers,
            event,
            event_type,
            fired_by,
            self.config.handler_timeout,
            |_, _| {},
        )
        .await;
    }

    async fn run_handlers(
        &self,
        handlers: Option<Handlers>,
        event: &mut (dyn Any + Send),
        event_type: &'static str,
        fired_by: &Arc<str>,
        timeout: Duration,
        mut after_each: impl FnMut(&HandlerEntry, &(dyn Any + Send)),
    ) {
        let Some(handlers) = handlers else {
            return;
        };
        let mut clock = Instant::now();
        for entry in handlers.iter() {
            clock = self
                .dispatch_one(entry, &mut *event, event_type, fired_by, timeout, clock)
                .await;
            after_each(entry, &*event);
        }
    }

    /// Dispatches a packet event to all handlers registered for the given packet.
    ///
    /// Handlers are invoked sequentially in priority order, same as `fire()`.
    pub async fn fire_packet_event(
        &self,
        packet_id: i32,
        state: ConnectionState,
        direction: PacketDirection,
        event: &mut RawPacketEvent,
    ) {
        let key = PacketKey {
            packet_id,
            state,
            direction,
        };
        let handlers = self.packet_handlers.snapshot(&key);
        self.run_handlers(
            handlers,
            event,
            type_name::<RawPacketEvent>(),
            &self.core_owner,
            self.config.packet_handler_timeout,
            |_, _| {},
        )
        .await;
    }

    async fn dispatch_one(
        &self,
        entry: &HandlerEntry,
        event: &mut (dyn Any + Send),
        event_type: &'static str,
        fired_by: &Arc<str>,
        timeout: Duration,
        started: Instant,
    ) -> Instant {
        if !entry.alive.load(Ordering::Acquire) {
            return started;
        }
        let failure = match &entry.kind {
            HandlerKind::Sync(handler) => catch_unwind(AssertUnwindSafe(|| handler(event)))
                .err()
                .map(|payload| DiagnosticKind::Panicked {
                    message: panic_message(payload.as_ref()),
                }),
            HandlerKind::Async(handler) => {
                match catch_unwind(AssertUnwindSafe(move || {
                    let event = event;
                    handler(event)
                })) {
                    Err(payload) => Some(DiagnosticKind::Panicked {
                        message: panic_message(payload.as_ref()),
                    }),
                    Ok(future) => {
                        let mut guarded = AssertUnwindSafe(future).catch_unwind();
                        let first =
                            std::future::poll_fn(|cx| Poll::Ready(guarded.poll_unpin(cx))).await;
                        let result = match first {
                            Poll::Ready(result) => Ok(result),
                            Poll::Pending => tokio::time::timeout_at(started + timeout, guarded)
                                .await
                                .map_err(|_| DiagnosticKind::TimedOut),
                        };
                        match result {
                            Ok(Ok(())) => None,
                            Ok(Err(payload)) => Some(DiagnosticKind::Panicked {
                                message: panic_message(payload.as_ref()),
                            }),
                            Err(timed_out) => Some(timed_out),
                        }
                    }
                }
            }
        };
        let finished = Instant::now();
        let elapsed = finished.saturating_duration_since(started);
        let kind = failure.or_else(|| {
            (elapsed > self.config.slow_handler_threshold).then_some(DiagnosticKind::Slow)
        });
        match kind {
            Some(kind) => {
                self.report(&entry.owner, event_type, fired_by, kind, elapsed);
                Instant::now()
            }
            None => finished,
        }
    }

    #[cold]
    fn report(
        &self,
        owner: &Arc<str>,
        event_type: &'static str,
        fired_by: &Arc<str>,
        kind: DiagnosticKind,
        elapsed: Duration,
    ) {
        let event = short_type_name(event_type);
        match &kind {
            DiagnosticKind::Panicked { message } => tracing::error!(
                plugin = %owner,
                event,
                fired_by = %fired_by,
                elapsed = ?elapsed,
                panic = %message,
                "event handler panicked; the event continues to the next handler"
            ),
            DiagnosticKind::TimedOut => tracing::error!(
                plugin = %owner,
                event,
                fired_by = %fired_by,
                elapsed = ?elapsed,
                "event handler timed out and was cancelled; the event continues to the next handler"
            ),
            DiagnosticKind::Slow => tracing::warn!(
                plugin = %owner,
                event,
                fired_by = %fired_by,
                elapsed = ?elapsed,
                threshold = ?self.config.slow_handler_threshold,
                "event handler is slow"
            ),
            DiagnosticKind::QueueFull => tracing::warn!(
                event,
                capacity = POSTED_EVENT_CAPACITY,
                "the posted event queue is full; posted events are dropped until it drains"
            ),
        }
        let _ = self.diagnostics.send(HandlerDiagnostic {
            owner: Arc::clone(owner),
            fired_by: Arc::clone(fired_by),
            event,
            kind,
            elapsed,
        });
    }

    pub(crate) fn subscribe_owned(
        &self,
        owner: Arc<str>,
        event_type: TypeId,
        priority: EventPriority,
        kind: HandlerKind,
    ) -> ListenerHandle {
        let entry = self.new_entry(owner, priority, kind);
        let handle = entry.handle;
        self.handlers.insert(event_type, entry);
        handle
    }

    pub(crate) fn subscribe_packet_owned(
        &self,
        owner: Arc<str>,
        filter: PacketFilter,
        priority: EventPriority,
        kind: HandlerKind,
    ) -> ListenerHandle {
        if filter.state != ConnectionState::Play {
            tracing::error!(
                plugin = %owner,
                packet_id = filter.packet_id,
                state = ?filter.state,
                "packet listeners only receive Play packets; this subscription is ignored"
            );
            return self.next_handle();
        }
        let key = PacketKey {
            packet_id: filter.packet_id,
            state: filter.state,
            direction: filter.direction,
        };
        let entry = self.new_entry(owner, priority, kind);
        let handle = entry.handle;
        self.packet_handlers.insert(key, entry);
        self.packet_listener_count.fetch_add(1, Ordering::Relaxed);
        handle
    }

    pub(crate) fn unsubscribe_owned(&self, owner: &str, handle: ListenerHandle) -> bool {
        if self.handlers.remove(owner, handle) {
            return true;
        }
        let removed = self.packet_handlers.remove(owner, handle);
        if removed {
            self.packet_listener_count.fetch_sub(1, Ordering::Relaxed);
        }
        removed
    }

    pub(crate) fn unsubscribe_owner(&self, owner: &str) -> usize {
        let packets = self.packet_handlers.remove_owner(owner);
        self.packet_listener_count
            .fetch_sub(packets as u64, Ordering::Relaxed);
        self.handlers.remove_owner(owner) + packets
    }

    pub(crate) fn listener_count(&self, owner: &str) -> usize {
        self.handlers.count_owner(owner) + self.packet_handlers.count_owner(owner)
    }

    fn new_entry(
        &self,
        owner: Arc<str>,
        priority: EventPriority,
        kind: HandlerKind,
    ) -> HandlerEntry {
        HandlerEntry {
            handle: self.next_handle(),
            priority,
            owner,
            alive: Arc::new(AtomicBool::new(true)),
            kind,
        }
    }

    /// Generates the next unique `ListenerHandle`.
    fn next_handle(&self) -> ListenerHandle {
        ListenerHandle::from_raw(self.next_handle.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for EventBusImpl {
    fn default() -> Self {
        Self::new()
    }
}

// Allow `infrarust-core` to implement the sealed `EventBus` trait.
impl infrarust_api::__sealed::Sealed for EventBusImpl {}

impl EventBus for EventBusImpl {
    fn subscribe_erased(
        &self,
        event_type: TypeId,
        priority: EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        self.subscribe_owned(
            Arc::clone(&self.core_owner),
            event_type,
            priority,
            HandlerKind::from_sync(handler),
        )
    }

    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        self.subscribe_owned(
            Arc::clone(&self.core_owner),
            event_type,
            priority,
            HandlerKind::from_async(handler),
        )
    }

    fn subscribe_packet(
        &self,
        filter: PacketFilter,
        priority: EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle {
        self.subscribe_packet_owned(
            Arc::clone(&self.core_owner),
            filter,
            priority,
            HandlerKind::from_sync(handler),
        )
    }

    fn subscribe_packet_async(
        &self,
        filter: PacketFilter,
        priority: EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        self.subscribe_packet_owned(
            Arc::clone(&self.core_owner),
            filter,
            priority,
            HandlerKind::from_async(handler),
        )
    }

    fn has_packet_listeners(
        &self,
        packet_id: i32,
        state: ConnectionState,
        direction: PacketDirection,
    ) -> bool {
        if self.packet_listener_count.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let key = PacketKey {
            packet_id,
            state,
            direction,
        };
        self.packet_handlers.has(&key)
    }

    fn unsubscribe(&self, handle: ListenerHandle) -> bool {
        self.unsubscribe_owned(CORE_OWNER, handle)
    }

    fn fire_erased<'a>(
        &'a self,
        event: &'a mut dyn ErasedEvent,
    ) -> BoxFuture<'a, Result<(), FireError>> {
        Box::pin(self.fire_from(&self.core_owner, event))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    struct TestEvent;
    impl Event for TestEvent {}

    fn noop_handler() -> ErasedHandler {
        Box::new(|_| {})
    }

    fn packet_filter() -> PacketFilter {
        PacketFilter {
            packet_id: 0x01,
            state: ConnectionState::Play,
            direction: PacketDirection::Serverbound,
        }
    }

    #[test]
    fn unsubscribe_prunes_empty_lifecycle_entry() {
        let bus = EventBusImpl::new();
        let keep = bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::NORMAL,
            noop_handler(),
        );
        let removed = bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::NORMAL,
            noop_handler(),
        );

        bus.unsubscribe(removed);
        assert_eq!(bus.handlers.len(), 1);

        bus.unsubscribe(keep);
        assert_eq!(bus.handlers.len(), 0);
    }

    #[test]
    fn unsubscribe_prunes_empty_packet_entry() {
        let bus = EventBusImpl::new();
        let handle = bus.subscribe_packet(packet_filter(), EventPriority::NORMAL, noop_handler());

        bus.unsubscribe(handle);
        assert_eq!(bus.packet_handlers.len(), 0);
        assert_eq!(bus.packet_listener_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn run_handlers_reports_each_handler_once_in_priority_order() {
        let bus = EventBusImpl::new();
        let last = bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::LAST,
            noop_handler(),
        );
        let first = bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::FIRST,
            noop_handler(),
        );
        let normal = bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::NORMAL,
            noop_handler(),
        );

        let mut seen = Vec::new();
        let handlers = bus.handlers.snapshot(&TypeId::of::<TestEvent>());
        bus.run_handlers(
            handlers,
            &mut TestEvent,
            type_name::<TestEvent>(),
            &bus.core_owner,
            Duration::from_secs(1),
            |entry, _| seen.push(entry.handle),
        )
        .await;

        assert_eq!(seen, vec![first, normal, last]);
    }

    #[test]
    fn unsubscribe_unknown_handle_is_noop() {
        let bus = EventBusImpl::new();
        bus.subscribe_erased(
            TypeId::of::<TestEvent>(),
            EventPriority::NORMAL,
            noop_handler(),
        );

        assert!(!bus.unsubscribe(ListenerHandle::from_raw(9999)));
        assert_eq!(bus.handlers.len(), 1);
    }

    #[test]
    fn packet_subscriptions_outside_play_are_refused() {
        let bus = EventBusImpl::new();
        let filter = PacketFilter {
            state: ConnectionState::Configuration,
            ..packet_filter()
        };
        let handle = bus.subscribe_packet(filter, EventPriority::NORMAL, noop_handler());

        assert!(!bus.has_packet_listeners(filter.packet_id, filter.state, filter.direction));
        assert!(!bus.unsubscribe(handle));
    }
}
