//! Ban storage trait definition.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

use uuid::Uuid;

use crate::ban::types::{BanEntry, BanTarget};
use crate::error::CoreError;

/// Backend for ban storage.
///
/// The trait is dyn-compatible, using `Pin<Box<dyn Future>>` return types.
/// Implemented by `FileBanStorage` (JSON file).
/// Future: Redis, `SQLite`.
pub trait BanStorage: Send + Sync {
    /// Adds a ban. If a ban already exists for this target, it is replaced.
    fn add_ban(
        &self,
        entry: BanEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>>;

    /// Removes a ban. Returns `true` if a ban existed.
    fn remove_ban(
        &self,
        target: &BanTarget,
    ) -> Pin<Box<dyn Future<Output = Result<bool, CoreError>> + Send + '_>>;

    /// Checks if a specific target is banned.
    /// Returns `None` if not banned or if the ban has expired.
    /// Expired bans are lazily purged.
    fn is_banned(
        &self,
        target: &BanTarget,
    ) -> Pin<Box<dyn Future<Output = Result<Option<BanEntry>, CoreError>> + Send + '_>>;

    /// Checks a player against all three ban types in one operation.
    /// Order: IP → username → UUID.
    /// Returns the first active ban found, or `None`.
    fn check_player<'a>(
        &'a self,
        ip: &'a IpAddr,
        username: &'a str,
        uuid: Option<&'a Uuid>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<BanEntry>, CoreError>> + Send + 'a>>;

    /// Lists all active (non-expired) bans.
    fn get_all_active(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<BanEntry>, CoreError>> + Send + '_>>;

    /// Purges expired bans. Returns the number of bans removed.
    fn purge_expired(&self) -> Pin<Box<dyn Future<Output = Result<usize, CoreError>> + Send + '_>>;

    /// Loads bans from the persistent backend (at startup).
    fn load(&self) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>>;

    /// Persists current state to the backend (for file-based backends).
    fn save(&self) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>>;
}
