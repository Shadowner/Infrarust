//! Virtual backend system (Tier 3).
//!
//! Virtual backends let plugins act as fully custom "servers" that speak
//! the Minecraft protocol directly to the client. This is the most powerful
//! plugin tier — handlers receive and send raw packets.
//!
//! - [`VirtualBackendHandler`] — plugin-implemented trait for handling sessions.
//! - [`VirtualBackendSession`] — proxy-provided session handle (sealed).

pub mod handler;
pub mod session;

pub use handler::VirtualBackendHandler;
pub use session::VirtualBackendSession;
