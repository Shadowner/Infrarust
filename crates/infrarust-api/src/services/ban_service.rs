//! Ban service.

use std::net::IpAddr;
use std::time::Duration;

use crate::error::ServiceError;
use crate::event::BoxFuture;

mod private {
    /// Sealed — only the proxy implements [`BanService`](super::BanService).
    pub trait Sealed {}
}

/// The target of a ban.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BanTarget {
    /// Ban by IP address.
    Ip(IpAddr),
    /// Ban by username.
    Username(String),
    /// Ban by Mojang UUID.
    Uuid(uuid::Uuid),
}

/// A recorded ban entry.
#[derive(Debug, Clone)]
pub struct BanEntry {
    /// What was banned.
    pub target: BanTarget,
    /// Optional reason for the ban.
    pub reason: Option<String>,
    /// When the ban expires, or `None` for permanent bans.
    pub expires_at: Option<std::time::SystemTime>,
    /// When the ban was created.
    pub created_at: std::time::SystemTime,
    /// Who or what issued the ban (e.g. plugin name, admin username).
    pub source: String,
}

/// Service for managing player bans.
///
/// Obtained via [`PluginContext::ban_service()`](crate::plugin::PluginContext::ban_service).
pub trait BanService: Send + Sync + private::Sealed {
    /// Bans a target with an optional reason and duration.
    ///
    /// A `None` duration means permanent ban.
    fn ban(
        &self,
        target: BanTarget,
        reason: Option<String>,
        duration: Option<Duration>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;

    /// Removes a ban. Returns `true` if a ban was removed.
    fn unban(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>>;

    /// Checks if a target is currently banned.
    fn is_banned(&self, target: &BanTarget) -> BoxFuture<'_, Result<bool, ServiceError>>;

    /// Returns the ban entry for a target, if any.
    fn get_ban(&self, target: &BanTarget) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>>;

    /// Returns all active bans.
    fn get_all_bans(&self) -> BoxFuture<'_, Result<Vec<BanEntry>, ServiceError>>;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn ban_target_non_exhaustive() {
        let target = BanTarget::Username("griefer".into());
        #[allow(unreachable_patterns)]
        match target {
            BanTarget::Ip(_) | BanTarget::Username(_) | BanTarget::Uuid(_) | _ => {}
        }
    }
}
