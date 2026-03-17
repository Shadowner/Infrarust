//! `EventBusImpl` — the dispatch engine for Infrarust events.
//!
//! Uses a snapshot pattern: handlers are stored in `Arc<Vec<HandlerEntry>>`
//! behind a `RwLock`. On dispatch, the `Arc` is cloned and the lock is
//! released before iterating handlers. This ensures async handlers never
//! hold a lock across `.await` points.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler, EventBus};
use infrarust_api::event::{Event, EventPriority, ListenerHandle};

use super::handler::{HandlerEntry, HandlerKind};

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

    /// Monotonic counter for generating unique `ListenerHandle` values.
    next_handle: AtomicU64,
}

impl EventBusImpl {
    /// Creates a new, empty event bus.
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        }
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
            for entry in handlers.iter() {
                match &entry.kind {
                    HandlerKind::Sync(handler) => {
                        handler(&mut event as &mut dyn Any);
                    }
                    HandlerKind::Async(handler) => {
                        handler(&mut event as &mut dyn Any).await;
                    }
                }
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

    /// Generates the next unique `ListenerHandle`.
    fn next_handle(&self) -> ListenerHandle {
        ListenerHandle::new(self.next_handle.fetch_add(1, Ordering::Relaxed))
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
        let entry = HandlerEntry {
            handle: self.next_handle(),
            priority,
            kind: HandlerKind::from_sync(handler),
        };
        self.insert_handler(event_type, entry)
    }

    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle {
        let entry = HandlerEntry {
            handle: self.next_handle(),
            priority,
            kind: HandlerKind::from_async(handler),
        };
        self.insert_handler(event_type, entry)
    }

    fn unsubscribe(&self, handle: ListenerHandle) {
        let mut map = self
            .handlers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for vec_arc in map.values_mut() {
            let vec = Arc::make_mut(vec_arc);
            if let Some(pos) = vec.iter().position(|h| h.handle == handle) {
                vec.remove(pos);
                return;
            }
        }
    }
}
