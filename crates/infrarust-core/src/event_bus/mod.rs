//! `EventBus` — typed event dispatch system for Infrarust.
//!
//! Provides sequential handler dispatch with priority ordering,
//! supporting both sync and async handlers. The bus uses a snapshot
//! pattern to avoid holding locks during async dispatch.

pub mod bus;
pub mod conversion;
pub(crate) mod handler;

pub use bus::EventBusImpl;
