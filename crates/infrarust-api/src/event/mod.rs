//! Event system infrastructure.
//!
//! Defines the core event traits, priority system, and the [`BoxFuture`] type
//! alias used for dyn-compatible async methods throughout the API.

pub mod bus;

use std::future::Future;
use std::pin::Pin;

/// A boxed, pinned, `Send` future — the standard return type for
/// async methods on dyn-compatible (sealed) traits.
///
/// Plugins never need to construct this directly; it is the return type
/// of proxy-provided async methods like [`Player::disconnect`](crate::player::Player::disconnect).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub use infrarust_plugin_common::EventPriority;

/// An opaque handle returned when subscribing to an event.
///
/// Use this with [`EventBus::unsubscribe`](bus::EventBus::unsubscribe) to
/// remove a listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerHandle(u64);

impl ListenerHandle {
    #[doc(hidden)]
    pub const fn from_raw(id: u64) -> Self {
        Self(id)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Connection state for packet filtering.
/// Mirror of `infrarust_protocol::version::ConnectionState` for API stability.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// Initial handshake state.
    Handshake,
    /// Server list ping / status query.
    Status,
    /// Authentication and login flow.
    Login,
    /// Configuration state (1.20.2+).
    Configuration,
    /// Main gameplay state.
    Play,
}

/// Filter for subscribing to a specific packet by ID, state, and direction.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct PacketFilter {
    /// The numeric packet ID.
    pub packet_id: i32,
    /// The connection state the packet belongs to.
    pub state: ConnectionState,
    /// The direction of the packet.
    pub direction: PacketDirection,
}

pub use infrarust_plugin_common::enums::PacketDirection;

/// Marker trait for all proxy events.
///
/// All event types implement this trait. Plugins subscribe to events
/// through the [`EventBus`](bus::EventBus).
pub trait Event: Send + Sync + 'static {}

/// An event whose outcome can be influenced by listeners.
///
/// Listeners can read and modify the result via [`result()`](ResultedEvent::result)
/// and [`set_result()`](ResultedEvent::set_result). The proxy consumes the final
/// result after all listeners have executed.
pub trait ResultedEvent: Event {
    /// The result type for this event. Must implement `Default` to provide
    /// a sensible initial outcome (typically "allowed" / "pass").
    type Result: Default + Send + Sync;

    /// Returns a reference to the current result.
    fn result(&self) -> &Self::Result;

    /// Sets the event result.
    fn set_result(&mut self, result: Self::Result);
}

macro_rules! resulted_event {
    ($event:ty, $result:ty) => {
        impl $crate::event::Event for $event {}

        impl $crate::event::ResultedEvent for $event {
            type Result = $result;

            fn result(&self) -> &Self::Result {
                &self.result
            }

            fn set_result(&mut self, result: Self::Result) {
                self.result = result;
            }
        }
    };
}

pub(crate) use resulted_event;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn priority_ordering() {
        assert!(EventPriority::FIRST < EventPriority::EARLY);
        assert!(EventPriority::EARLY < EventPriority::NORMAL);
        assert!(EventPriority::NORMAL < EventPriority::LATE);
        assert!(EventPriority::LATE < EventPriority::LAST);
    }

    #[test]
    fn priority_custom() {
        let p = EventPriority::custom(100);
        assert!(EventPriority::EARLY < p);
        assert!(p < EventPriority::NORMAL);
    }

    #[test]
    fn listener_handle() {
        let h = ListenerHandle::from_raw(42);
        assert_eq!(h.as_u64(), 42);
    }
}
