//! Event bus trait for subscribing to proxy events.

use std::any::{Any, TypeId};

use super::{BoxFuture, EventPriority, ListenerHandle};

mod private {
    /// Sealed trait marker — only the proxy may implement [`EventBus`](super::EventBus).
    pub trait Sealed {}
}

/// A type-erased synchronous event handler.
pub type ErasedHandler = Box<dyn Fn(&mut dyn Any) + Send + Sync>;

/// A type-erased asynchronous event handler.
pub type ErasedAsyncHandler = Box<dyn Fn(&mut dyn Any) -> BoxFuture<'_, ()> + Send + Sync>;

/// The event bus allows plugins to subscribe to proxy events.
///
/// Obtained via [`PluginContext::event_bus()`](crate::plugin::PluginContext::event_bus).
/// The proxy is the sole implementor of this trait.
///
/// Use the typed convenience methods [`subscribe`](EventBusExt::subscribe) and
/// [`subscribe_async`](EventBusExt::subscribe_async) from the [`EventBusExt`]
/// trait (automatically in scope via the prelude).
///
/// # Example
/// ```ignore
/// use infrarust_api::prelude::*;
///
/// // In Plugin::on_enable:
/// ctx.event_bus().subscribe::<PostLoginEvent, _>(
///     EventPriority::NORMAL,
///     |event| {
///         tracing::info!("Player joined: {}", event.profile.username);
///     },
/// );
/// ```
pub trait EventBus: Send + Sync + private::Sealed {
    /// Subscribes a type-erased synchronous handler.
    ///
    /// Prefer the typed [`EventBusExt::subscribe`] method instead.
    fn subscribe_erased(
        &self,
        event_type: TypeId,
        priority: EventPriority,
        handler: ErasedHandler,
    ) -> ListenerHandle;

    /// Subscribes a type-erased asynchronous handler.
    ///
    /// Prefer the typed [`EventBusExt::subscribe_async`] method instead.
    fn subscribe_async_erased(
        &self,
        event_type: TypeId,
        priority: EventPriority,
        handler: ErasedAsyncHandler,
    ) -> ListenerHandle;

    /// Removes a previously registered listener.
    fn unsubscribe(&self, handle: ListenerHandle);
}

/// Extension trait providing typed event subscription methods.
///
/// This is automatically available on `&dyn EventBus` and is re-exported
/// in the prelude.
pub trait EventBusExt {
    /// Subscribes a synchronous handler for events of type `E`.
    ///
    /// The handler is called with a mutable reference to the event,
    /// allowing it to inspect and modify the event (including its result
    /// for [`ResultedEvent`](super::ResultedEvent) types).
    ///
    /// Returns a [`ListenerHandle`] that can be used to unsubscribe.
    fn subscribe<E, F>(&self, priority: EventPriority, handler: F) -> ListenerHandle
    where
        E: super::Event,
        F: Fn(&mut E) + Send + Sync + 'static;

    /// Subscribes an asynchronous handler for events of type `E`.
    ///
    /// The handler returns a [`BoxFuture`] that the proxy
    /// will await before calling the next listener.
    ///
    /// Returns a [`ListenerHandle`] that can be used to unsubscribe.
    fn subscribe_async<E, F>(&self, priority: EventPriority, handler: F) -> ListenerHandle
    where
        E: super::Event,
        F: Fn(&mut E) -> BoxFuture<'_, ()> + Send + Sync + 'static;
}

impl EventBusExt for dyn EventBus + '_ {
    fn subscribe<E, F>(&self, priority: EventPriority, handler: F) -> ListenerHandle
    where
        E: super::Event,
        F: Fn(&mut E) + Send + Sync + 'static,
    {
        self.subscribe_erased(
            TypeId::of::<E>(),
            priority,
            Box::new(move |any| {
                if let Some(event) = any.downcast_mut::<E>() {
                    handler(event);
                }
            }),
        )
    }

    fn subscribe_async<E, F>(&self, priority: EventPriority, handler: F) -> ListenerHandle
    where
        E: super::Event,
        F: Fn(&mut E) -> BoxFuture<'_, ()> + Send + Sync + 'static,
    {
        self.subscribe_async_erased(
            TypeId::of::<E>(),
            priority,
            Box::new(move |any| {
                if let Some(event) = any.downcast_mut::<E>() {
                    handler(event)
                } else {
                    Box::pin(async {})
                }
            }),
        )
    }
}
