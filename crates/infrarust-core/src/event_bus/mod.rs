//! `EventBus` — typed event dispatch system for Infrarust.
//!
//! Provides sequential handler dispatch with priority ordering,
//! supporting both sync and async handlers. The bus uses a snapshot
//! pattern to avoid holding locks during async dispatch.

pub mod builtin;
pub mod bus;
pub mod diagnostic;
pub(crate) mod handler;

#[cfg(feature = "test-support")]
pub use builtin::{BUILTIN_EVENTS, is_builtin_event};
pub use bus::{CORE_OWNER, EventBusConfig, EventBusImpl};
#[cfg(feature = "test-support")]
pub use diagnostic::{DiagnosticKind, HandlerDiagnostic};
