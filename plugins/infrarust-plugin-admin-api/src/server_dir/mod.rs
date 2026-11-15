//! `<data_dir>/servers/` — the TOML files this plugin owns.
//!
//! Every server created through the REST API lives here as a full
//! `infrarust_config::ServerConfig` document. The proxy's own `servers_dir` is
//! never touched. Changes reach the proxy as [`ServerDocument`] events, either
//! from the filesystem watcher or from an explicit reload.

mod legacy;
mod store;
#[cfg(any(test, feature = "test-util"))]
pub mod test_support;
#[cfg(test)]
mod tests;
mod watch;

pub use store::{
    Committed, DirChange, DocumentId, Ownership, ServerDir, StoreError, WriteLock, parse_document,
    to_document_text,
};
pub use watch::{ProviderSenderSlot, ReloadSummary, reload, watch};
