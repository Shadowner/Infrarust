//! Permission system types.
//!
//! Two-level permission model: [`Player`](PermissionLevel::Player) (no access by default)
//! and [`Admin`](PermissionLevel::Admin) (full access). Plugins can provide custom
//! [`PermissionChecker`] implementations via the [`PermissionsSetupEvent`](crate::events::lifecycle::PermissionsSetupEvent).

/// Permission level assigned to a player.
///
/// `Player < Admin` — used for access control on proxy commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionLevel {
    /// Default level — no proxy command access unless explicitly opened.
    Player,
    /// Full proxy command access.
    Admin,
}

/// Determines a player's permission level and checks named permissions.
///
/// The proxy provides a default implementation based on config (`[permissions].admins`).
/// Plugins can replace it per-player via
/// [`PermissionsSetupEvent`](crate::events::lifecycle::PermissionsSetupEvent).
pub trait PermissionChecker: Send + Sync {
    /// Returns the player's permission level.
    fn permission_level(&self) -> PermissionLevel;

    /// Checks a named permission string (e.g., `"infrarust.admin"`).
    fn has_permission(&self, permission: &str) -> bool;
}

/// Default permission checker — always [`Player`](PermissionLevel::Player), no permissions.
///
/// Used for passthrough sessions, offline-mode players, and tests.
pub struct DefaultPermissionChecker;

impl PermissionChecker for DefaultPermissionChecker {
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Player
    }

    fn has_permission(&self, _permission: &str) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_level_ordering() {
        assert!(PermissionLevel::Player < PermissionLevel::Admin);
    }

    #[test]
    fn default_checker_is_player() {
        let checker = DefaultPermissionChecker;
        assert_eq!(checker.permission_level(), PermissionLevel::Player);
        assert!(!checker.has_permission("infrarust.admin"));
        assert!(!checker.has_permission("anything"));
    }
}
