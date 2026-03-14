//! Ban system for Infrarust.
//!
//! Provides IP, username, and UUID-based banning with file persistence,
//! audit logging, and runtime kick of connected players.

pub mod file_storage;
pub mod manager;
pub mod storage;
pub mod types;

pub use file_storage::FileBanStorage;
pub use manager::BanManager;
pub use storage::BanStorage;
pub use types::{BanAction, BanAuditLogEntry, BanEntry, BanTarget};
