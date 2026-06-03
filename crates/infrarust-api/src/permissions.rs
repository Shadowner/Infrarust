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

/// A capability granted to a plugin.
///
/// The source of truth is the Infrarust config (`[plugins.<id>] permissions = [...]`);
/// compiled-in native plugins are trusted and receive [`CapabilitySet::native_trusted`].
/// This is the plugin-capability model and is unrelated to the player/role
/// [`PermissionLevel`] above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Subscribe to domain events (lifecycle, connection, proxy, chat).
    EventBus,
    /// Read the player registry and player state.
    PlayerRead,
    /// Act on a player (message, title, kick, switch-server).
    PlayerWrite,
    /// Emit raw packets and receive `RawPacketEvent`.
    RawPacket,
    /// Start/stop servers and read their state.
    ServerManage,
    /// Use the ban service.
    Ban,
    /// Register commands.
    Command,
    /// Schedule tasks.
    Scheduler,
    /// Read the proxy configuration.
    ConfigRead,
    /// Register codec filters (WASM: gated; native: implicit).
    CodecFilter,
    TransportFilter,
    /// Provide limbo handlers.
    Limbo,
    /// Provide virtual backends (Tier 3).
    VirtualBackend,
    /// Provide a custom permission checker.
    PermissionProvider,
    /// Filesystem access beyond the dedicated `data_dir`.
    FilesystemExtended,
    /// Outbound network access.
    Network,
}

impl Capability {
    pub const ALL: [Capability; 16] = [
        Capability::EventBus,
        Capability::PlayerRead,
        Capability::PlayerWrite,
        Capability::RawPacket,
        Capability::ServerManage,
        Capability::Ban,
        Capability::Command,
        Capability::Scheduler,
        Capability::ConfigRead,
        Capability::CodecFilter,
        Capability::TransportFilter,
        Capability::Limbo,
        Capability::VirtualBackend,
        Capability::PermissionProvider,
        Capability::FilesystemExtended,
        Capability::Network,
    ];

    #[must_use]
    pub fn to_kebab(self) -> &'static str {
        match self {
            Capability::EventBus => "event-bus",
            Capability::PlayerRead => "player-read",
            Capability::PlayerWrite => "player-write",
            Capability::RawPacket => "raw-packet",
            Capability::ServerManage => "server-manage",
            Capability::Ban => "ban",
            Capability::Command => "command",
            Capability::Scheduler => "scheduler",
            Capability::ConfigRead => "config-read",
            Capability::CodecFilter => "codec-filter",
            Capability::TransportFilter => "transport-filter",
            Capability::Limbo => "limbo",
            Capability::VirtualBackend => "virtual-backend",
            Capability::PermissionProvider => "permission-provider",
            Capability::FilesystemExtended => "filesystem-extended",
            Capability::Network => "network",
        }
    }

    #[must_use]
    pub fn from_kebab(s: &str) -> Option<Capability> {
        let cap = match s {
            "event-bus" => Capability::EventBus,
            "player-read" => Capability::PlayerRead,
            "player-write" => Capability::PlayerWrite,
            "raw-packet" => Capability::RawPacket,
            "server-manage" => Capability::ServerManage,
            "ban" => Capability::Ban,
            "command" => Capability::Command,
            "scheduler" => Capability::Scheduler,
            "config-read" => Capability::ConfigRead,
            "codec-filter" => Capability::CodecFilter,
            "transport-filter" => Capability::TransportFilter,
            "limbo" => Capability::Limbo,
            "virtual-backend" => Capability::VirtualBackend,
            "permission-provider" => Capability::PermissionProvider,
            "filesystem-extended" => Capability::FilesystemExtended,
            "network" => Capability::Network,
            _ => return None,
        };
        Some(cap)
    }
}

/// The set of [`Capability`]s granted to a plugin.
#[derive(Debug, Clone, Default)]
pub struct CapabilitySet {
    granted: std::collections::HashSet<Capability>,
}

impl CapabilitySet {
    #[must_use]
    pub fn has(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }

    pub fn insert(&mut self, cap: Capability) {
        self.granted.insert(cap);
    }

    #[must_use]
    pub fn with(mut self, cap: Capability) -> Self {
        self.granted.insert(cap);
        self
    }

    #[must_use]
    pub fn baseline() -> Self {
        Self::default()
            .with(Capability::EventBus)
            .with(Capability::PlayerRead)
            .with(Capability::PlayerWrite)
            .with(Capability::Command)
            .with(Capability::Scheduler)
            .with(Capability::ConfigRead)
    }

    #[must_use]
    pub fn native_trusted() -> Self {
        let mut set = Self::default();
        for cap in Capability::ALL {
            set.insert(cap);
        }
        set
    }

    #[must_use]
    pub fn from_config_strings(strings: &[String]) -> (Self, Vec<String>) {
        let mut set = Self::baseline();
        let mut rejected = Vec::new();
        for s in strings {
            match Capability::from_kebab(s) {
                Some(Capability::TransportFilter) | None => rejected.push(s.clone()),
                Some(cap) => set.insert(cap),
            }
        }
        (set, rejected)
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

    #[test]
    fn capability_kebab_roundtrips_for_every_variant() {
        use std::collections::HashSet;
        let mut names = HashSet::new();
        for cap in Capability::ALL {
            let s = cap.to_kebab();
            assert!(names.insert(s), "duplicate kebab name: {s}");
            assert_eq!(Capability::from_kebab(s), Some(cap), "from_kebab lost {cap:?}");
        }
        assert_eq!(Capability::ALL.len(), 16);
        assert_eq!(names.len(), Capability::ALL.len());
    }

    #[test]
    fn from_kebab_rejects_unknown() {
        assert_eq!(Capability::from_kebab("nope"), None);
        assert_eq!(Capability::from_kebab(""), None);
        assert_eq!(Capability::from_kebab("codec_filter"), None); // snake, not kebab
    }

    #[test]
    fn baseline_contains_only_default_grants() {
        let b = CapabilitySet::baseline();
        for cap in [
            Capability::EventBus,
            Capability::PlayerRead,
            Capability::PlayerWrite,
            Capability::Command,
            Capability::Scheduler,
            Capability::ConfigRead,
        ] {
            assert!(b.has(cap), "baseline missing {cap:?}");
        }
        assert!(!b.has(Capability::Ban));
        assert!(!b.has(Capability::CodecFilter));
        assert!(!b.has(Capability::TransportFilter));
    }

    #[test]
    fn native_trusted_contains_every_capability() {
        let t = CapabilitySet::native_trusted();
        for cap in Capability::ALL {
            assert!(t.has(cap), "native_trusted missing {cap:?}");
        }
        assert!(t.has(Capability::TransportFilter));
    }

    #[test]
    fn from_config_strings_adds_opt_ins_over_baseline() {
        let (set, rejected) = CapabilitySet::from_config_strings(&[
            "server-manage".to_string(),
            "codec-filter".to_string(),
        ]);
        assert!(set.has(Capability::ServerManage));
        assert!(set.has(Capability::CodecFilter));
        assert!(set.has(Capability::EventBus), "baseline must be preserved");
        assert!(rejected.is_empty());
    }

    #[test]
    fn from_config_strings_collects_unknown_and_refuses_transport_filter() {
        let (set, rejected) = CapabilitySet::from_config_strings(&[
            "ban-all".to_string(),          // unknown
            "transport-filter".to_string(), // known but never grantable via config
            "ban".to_string(),              // valid opt-in
        ]);
        assert!(set.has(Capability::Ban));
        assert!(!set.has(Capability::TransportFilter));
        assert_eq!(rejected.len(), 2);
        assert!(rejected.contains(&"ban-all".to_string()));
        assert!(rejected.contains(&"transport-filter".to_string()));
    }
}
