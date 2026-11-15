//! Ban system for Infrarust.
//!
//! Provides IP, username, and UUID-based banning with file persistence,
//! audit logging, and runtime kick of connected players.

pub mod builtin;
pub mod file_storage;
pub mod manager;
pub mod types;

#[cfg(feature = "test-support")]
pub use file_storage::FileBanStorage;
#[cfg(feature = "test-support")]
pub use manager::BAN_CHECK_UNAVAILABLE;
pub use manager::BanManager;
