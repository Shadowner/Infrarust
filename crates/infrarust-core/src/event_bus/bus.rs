//! `EventBusImpl` — the dispatch engine for Infrarust events.
//!
//! Uses a snapshot pattern: handlers are stored in `Arc<Vec<HandlerEntry>>`
//! behind a `RwLock`. On dispatch, the `Arc` is cloned and the lock is
//! released before iterating handlers. This ensures async handlers never
//! hold a lock across `.await` points.

use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::task::Poll;
use std::time::Duration;

use futures_util::FutureExt;
use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler, EventBus};
use infrarust_api::event::{
    ConnectionState, Event, EventPriority, ListenerHandle, PacketDirection, PacketFilter,
};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_config::EventsConfig;
use tokio::sync::broadcast;
use tokio::time::Instant;

use super::diagnostic::{DiagnosticKind, HandlerDiagnostic, panic_message, short_type_name};
use super::handler::{HandlerEntry, HandlerKind};

pub const CORE_OWNER: &str = "infrarust";

const DIAGNOSTIC_CAPACITY: usize = 256;

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

/// Internal key for packet-specific handler lookup.
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
struct PacketKey {
    packet_id: i32,
    state: ConnectionState,
    direction: PacketDirection,
}

/// The proxy's event bus implementation.
///
/// Provides sequential handler dispatch with priority ordering,
/// supporting both synchronous and asynchronous handlers. The bus uses
/// a snapshot pattern (`Arc<Vec>`) to avoid holding locks during dispatch.
///
/// # Dispatch semantics
///
/// - Handlers are invoked in priority order: `FIRST` (0) runs first,
///   `LAST` (255) runs last.
/// - Each handler sees the modifications made by previous handlers.
/// - Sync handlers run inline; async handlers are `.await`ed sequentially.
///
/// # Thread safety
///
/// `subscribe` and `unsubscribe` take a short write lock (~200 ns).
/// `fire` takes a short read lock (~50 ns) then dispatches without any lock.
/// A `subscribe` during an in-progress `fire` uses copy-on-write via
/// `Arc::make_mut` — the running dispatch continues on the old snapshot.
pub struct EventBusImpl {
    /// Handlers grouped by event `TypeId`.
    /// The `Arc<Vec<...>>` enables lock-free dispatch via snapshot cloning.
    handlers: RwLock<HashMap<TypeId, Arc<Vec<HandlerEntry>>>>,

    /// Handlers for specific packet (id, state, direction) combinations.
    packet_handlers: RwLock<HashMap<PacketKey, Arc<Vec<HandlerEntry>>>>,

    /// Monotonic counter for generating unique `ListenerHandle` values.
    next_handle: AtomicU64,
    packet_listener_count: AtomicU64,
    config: EventBusConfig,
    diagnostics: broadcast::Sender<HandlerDiagnostic>,
    core_owner: Arc<str>,
}

impl EventBusImpl {
    pub fn new() -> Self {
        Self::with_config(EventBusConfig::default())
    }

    pub fn with_config(config: EventBusConfig) -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
            packet_handlers: RwLock::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
            packet_listener_count: AtomicU64::new(0),
            config,
            diagnostics: broadcast::channel(DIAGNOSTIC_CAPACITY).0,
            core_owner: Arc::from(CORE_OWNER),
        }
    }

    pub const fn config(&self) -> EventBusConfig {
        self.config
    }

    pub fn diagnostics(&self) -> broadcast::Receiver<HandlerDiagnostic> {
        self.diagnostics.subscribe()
    }

    pub fn listener_owners<E: Event>(&self) -> Vec<Arc<str>> {
        let map = self
            .handlers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&TypeId::of::<E>())
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
        let type_id = TypeId::of::<E>();

        // Snapshot: clone the Arc, then release the lock immediately.
        let snapshot = {
            let map = self
                .handlers
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.get(&type_id).cloned()
        };

        if let Some(handlers) = snapshot {
            let mut clock = Instant::now();
            for entry in handlers.iter() {
                clock = self
                    .dispatch_one(
                        entry,
                        &mut event,
                        type_name::<E>(),
                        self.config.handler_timeout,
                        clock,
                    )
                    .await;
            }
        }

        event
    }

    /// Dispatches an event in a detached tokio task (fire-and-forget).
    ///
    /// The caller cannot observe the event after this call. Used for
    /// informational events like `ServerStateChangeEvent` and
    /// `ConfigReloadEvent`.
    pub fn fire_and_forget_arc<E: Event + Send + 'static>(self: &Arc<Self>, event: E) {
        let bus = Arc::clone(self);
        tokio::spawn(async move {
            let _ = bus.fire(event).await;
        });
    }

    /// Internal helper: inserts a handler entry into the sorted vec for
    /// the given event type.
    #[allow(clippy::significant_drop_tightening)] // map is used for multiple ops on vec_arc
    fn insert_handler(&self, event_type: TypeId, entry: HandlerEntry) -> ListenerHandle {
        let handle = entry.handle;
        {
            let mut map = self
                .handlers
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let vec_arc = map.entry(event_type).or_default();

            // Copy-on-write: if a dispatch holds the old Arc, this clones the Vec.
            let vec = Arc::make_mut(vec_arc);

            // Insert sorted by priority ascending (FIRST=0 first, LAST=255 last).
            // partition_point finds the first index where priority > entry's priority,
            // ensuring same-priority handlers preserve insertion order.
            let pos = vec.partition_point(|h| h.priority.value() <= entry.priority.value());
            vec.insert(pos, entry);
        }
        handle
    }

    /// Internal helper: inserts a handler entry into the sorted vec for
    /// the given packet key.
    #[allow(clippy::significant_drop_tightening)]
    fn insert_packet_handler(&self, key: PacketKey, entry: HandlerEntry) -> ListenerHandle {
        let handle = entry.handle;
        {
            let mut map = self
                .packet_handlers
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let vec_arc = map.entry(key).or_default();
            let vec = Arc::make_mut(vec_arc);
            let pos = vec.partition_point(|h| h.priority.value() <= entry.priority.value());
            vec.insert(pos, entry);
        }
        self.packet_listener_count.fetch_add(1, Ordering::Relaxed);
        handle
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
        let snapshot = {
            let map = self
                .packet_handlers
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.get(&key).cloned()
        };

        if let Some(handlers) = snapshot {
            let mut clock = Instant::now();
            for entry in handlers.iter() {
                clock = self
                    .dispatch_one(
                        entry,
                        &mut *event,
                        type_name::<RawPacketEvent>(),
                        self.config.packet_handler_timeout,
                        clock,
                    )
                    .await;
            }
        }
    }

    async fn dispatch_one(
        &self,
        entry: &HandlerEntry,
        event: &mut (dyn Any + Send),
        event_type: &'static str,
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
                self.report(entry, event_type, kind, elapsed);
                Instant::now()
            }
            None => finished,
        }
    }

    #[cold]
    fn report(
        &self,
        entry: &HandlerEntry,
        event_type: &'static str,
        kind: DiagnosticKind,
        elapsed: Duration,
    ) {
        let event = short_type_name(event_type);
        match &kind {
            DiagnosticKind::Panicked { message } => tracing::error!(
                plugin = %entry.owner,
                event,
                elapsed = ?elapsed,
                panic = %message,
                "event handler panicked; the event continues to the next handler"
            ),
            DiagnosticKind::TimedOut => tracing::error!(
                plugin = %entry.owner,
                event,
                elapsed = ?elapsed,
                "event handler timed out and was cancelled; the event continues to the next handler"
            ),
            DiagnosticKind::Slow => tracing::warn!(
                plugin = %entry.owner,
                event,
                elapsed = ?elapsed,
                threshold = ?self.config.slow_handler_threshold,
                "event handler is slow"
            ),
        }
        let _ = self.diagnostics.send(HandlerDiagnostic {
            owner: Arc::clone(&entry.owner),
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
        self.insert_handler(event_type, entry)
    }

    pub(crate) fn subscribe_packet_owned(
        &self,
        owner: Arc<str>,
        filter: PacketFilter,
        priority: EventPriority,
        kind: HandlerKind,
    ) -> ListenerHandle {
        let key = PacketKey {
            packet_id: filter.packet_id,
            state: filter.state,
            direction: filter.direction,
        };
        let entry = self.new_entry(owner, priority, kind);
        self.insert_packet_handler(key, entry)
    }

    pub(crate) fn unsubscribe_owned(&self, owner: &str, handle: ListenerHandle) -> bool {
        {
            let mut map = self
                .handlers
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if remove_handler(&mut map, owner, handle) {
                return true;
            }
        }
        let mut map = self
            .packet_handlers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let removed = remove_handler(&mut map, owner, handle);
        if removed {
            self.packet_listener_count.fetch_sub(1, Ordering::Relaxed);
        }
        removed
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
impl infrarust_api::event::bus::private::Sealed for EventBusImpl {}

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
        let map = self
            .packet_handlers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(&key).is_some_and(|v| !v.is_empty())
    }

    fn unsubscribe(&self, handle: ListenerHandle) -> bool {
        self.unsubscribe_owned(CORE_OWNER, handle)
    }
}

fn remove_handler<K: Copy + Eq + std::hash::Hash>(
    map: &mut HashMap<K, Arc<Vec<HandlerEntry>>>,
    owner: &str,
    handle: ListenerHandle,
) -> bool {
    let Some((key, pos)) = map.iter().find_map(|(k, v)| {
        v.iter()
            .position(|h| h.handle == handle && *h.owner == *owner)
            .map(|p| (*k, p))
    }) else {
        return false;
    };

    if map.get(&key).is_some_and(|v| v.len() == 1) {
        if let Some(entries) = map.remove(&key) {
            entries[pos].alive.store(false, Ordering::Release);
        }
    } else if let Some(vec_arc) = map.get_mut(&key) {
        vec_arc[pos].alive.store(false, Ordering::Release);
        Arc::make_mut(vec_arc).remove(pos);
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
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
        assert_eq!(bus.handlers.read().unwrap().len(), 1);

        bus.unsubscribe(keep);
        assert!(bus.handlers.read().unwrap().is_empty());
    }

    #[test]
    fn unsubscribe_prunes_empty_packet_entry() {
        let bus = EventBusImpl::new();
        let handle = bus.subscribe_packet(packet_filter(), EventPriority::NORMAL, noop_handler());

        bus.unsubscribe(handle);
        assert!(bus.packet_handlers.read().unwrap().is_empty());
        assert_eq!(bus.packet_listener_count.load(Ordering::Relaxed), 0);
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
        assert_eq!(bus.handlers.read().unwrap().len(), 1);
    }
}
