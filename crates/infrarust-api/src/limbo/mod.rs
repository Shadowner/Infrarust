//! Limbo system (Tier 2).
//!
//! Limbo handlers provide game logic for proxy-hosted "waiting rooms"
//! where the proxy handles the Minecraft protocol and the plugin handles
//! player interaction (authentication, queues, server wake-up, etc.).
//!
//! - [`LimboHandler`] — plugin-implemented trait for handling players in limbo.
//! - [`LimboSession`] — proxy-provided session handle (sealed).
//! - [`HandlerResult`] — the outcome of a limbo handler action.

pub mod context;
pub mod handler;
pub mod session;

pub use context::LimboEntryContext;
pub use handler::{HandlerResult, LimboHandler};
pub use session::LimboSession;
