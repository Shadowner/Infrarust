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

/// Priority level for event listeners.
///
/// Listeners are invoked in order from highest priority (FIRST) to lowest
/// (LAST). Each listener sees the modifications made by previous listeners.
///
/// # Example
/// ```
/// use infrarust_api::event::EventPriority;
///
/// assert!(EventPriority::FIRST < EventPriority::LAST);
/// assert!(EventPriority::EARLY < EventPriority::NORMAL);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventPriority(u8);

impl EventPriority {
    /// Runs first — before all other listeners.
    pub const FIRST: Self = Self(0);
    /// Runs early — before normal listeners.
    pub const EARLY: Self = Self(64);
    /// Default priority.
    pub const NORMAL: Self = Self(128);
    /// Runs late — after normal listeners.
    pub const LATE: Self = Self(192);
    /// Runs last — after all other listeners.
    pub const LAST: Self = Self(255);

    /// Creates a custom priority value.
    pub const fn custom(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw priority value.
    pub const fn value(self) -> u8 {
        self.0
    }
}

/// An opaque handle returned when subscribing to an event.
///
/// Use this with [`EventBus::unsubscribe`](bus::EventBus::unsubscribe) to
/// remove a listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerHandle(u64);

impl ListenerHandle {
    /// Creates a new listener handle from a raw ID.
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw handle ID.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

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
        let h = ListenerHandle::new(42);
        assert_eq!(h.as_u64(), 42);
    }
}
