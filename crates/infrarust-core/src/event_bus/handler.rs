//! Internal handler types for the event bus.
//!
//! These types are `pub(crate)` — not exposed outside `infrarust-core`.

use std::any::Any;
use std::sync::Arc;

use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler};
use infrarust_api::event::{BoxFuture, EventPriority, ListenerHandle};

/// A shareable synchronous handler.
type SharedSyncHandler = Arc<dyn Fn(&mut dyn Any) + Send + Sync>;

/// A shareable asynchronous handler.
type SharedAsyncHandler = Arc<dyn Fn(&mut dyn Any) -> BoxFuture<'_, ()> + Send + Sync>;

/// Discriminant for synchronous vs asynchronous handlers.
///
/// Uses `Arc` internally so that `HandlerEntry` is `Clone` —
/// required by `Arc::make_mut` for the copy-on-write snapshot pattern.
#[derive(Clone)]
pub enum HandlerKind {
    /// A synchronous handler: `Fn(&mut dyn Any) + Send + Sync`.
    Sync(SharedSyncHandler),
    /// An asynchronous handler: `Fn(&mut dyn Any) -> BoxFuture<'_, ()> + Send + Sync`.
    Async(SharedAsyncHandler),
}

impl HandlerKind {
    /// Wraps the API's boxed sync handler into a shareable `Arc`.
    pub fn from_sync(handler: ErasedHandler) -> Self {
        Self::Sync(Arc::from(handler))
    }

    /// Wraps the API's boxed async handler into a shareable `Arc`.
    pub fn from_async(handler: ErasedAsyncHandler) -> Self {
        Self::Async(Arc::from(handler))
    }
}

/// A single registered handler with its metadata.
#[derive(Clone)]
pub struct HandlerEntry {
    /// Opaque handle for unsubscribe.
    pub handle: ListenerHandle,
    /// Dispatch priority (lower value = higher priority = runs first).
    pub priority: EventPriority,
    /// The type-erased handler closure.
    pub kind: HandlerKind,
}
