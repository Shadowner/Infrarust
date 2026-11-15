use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use infrarust_api::event::bus::{ErasedAsyncHandler, ErasedHandler};
use infrarust_api::event::{BoxFuture, EventPriority, ListenerHandle};

type SharedSyncHandler = Arc<dyn Fn(&mut dyn Any) + Send + Sync>;

type SharedAsyncHandler = Arc<dyn Fn(&mut dyn Any) -> BoxFuture<'_, ()> + Send + Sync>;

#[derive(Clone)]
pub enum HandlerKind {
    Sync(SharedSyncHandler),
    Async(SharedAsyncHandler),
}

impl HandlerKind {
    pub fn from_sync(handler: ErasedHandler) -> Self {
        Self::Sync(Arc::from(handler))
    }

    pub fn from_async(handler: ErasedAsyncHandler) -> Self {
        Self::Async(Arc::from(handler))
    }
}

#[derive(Clone)]
pub struct HandlerEntry {
    pub handle: ListenerHandle,
    pub priority: EventPriority,
    pub owner: Arc<str>,
    pub alive: Arc<AtomicBool>,
    pub kind: HandlerKind,
}
