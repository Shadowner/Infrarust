//! Limbo Engine — virtual Minecraft world with no backend server.
//!
//! Manages player sessions in an empty world, controlled by a chain of
//! [`LimboHandler`](infrarust_api::limbo::LimboHandler) plugins.

pub(crate) mod virtual_session; // VirtualSessionCore — shared plumbing
pub(crate) mod chunk;           // Empty chunk encoding
pub(crate) mod spawn;           // Spawn sequence (version-branched)
pub(crate) mod keepalive;       // KeepAlive state machine
pub(crate) mod chat;            // Client message parsing
pub(crate) mod session;         // LimboSessionImpl
pub(crate) mod handler_chain;   // Limbo-specific dispatch loop
pub(crate) mod registry;        // LimboHandlerRegistry
pub(crate) mod engine;          // enter_limbo() orchestrator
pub(crate) mod login;           // Login without backend

#[cfg(test)]
mod test_helpers;               // Shared test utilities
