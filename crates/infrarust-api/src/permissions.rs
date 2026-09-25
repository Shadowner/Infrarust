use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::event::BoxFuture;
use crate::types::{GameProfile, PlayerId};

pub const ADMIN_PERMISSION: &str = "infrarust.admin";
pub const COMMAND_PERMISSION_PREFIX: &str = "infrarust.command.";
pub const WILDCARD: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tristate {
    True,
    False,
    #[default]
    Undefined,
}

impl Tristate {
    #[must_use]
    pub const fn from_bool(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    #[must_use]
    pub const fn as_bool(self) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Undefined => None,
        }
    }

    #[must_use]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    #[must_use]
    pub const fn is_defined(self) -> bool {
        !matches!(self, Self::Undefined)
    }

    #[must_use]
    pub const fn or(self, fallback: Self) -> Self {
        match self {
            Self::Undefined => fallback,
            defined => defined,
        }
    }
}

impl From<bool> for Tristate {
    fn from(value: bool) -> Self {
        Self::from_bool(value)
    }
}

impl From<Option<bool>> for Tristate {
    fn from(value: Option<bool>) -> Self {
        value.map_or(Self::Undefined, Self::from_bool)
    }
}

pub trait PermissionChecker: Send + Sync {
    fn value(&self, node: &str) -> Tristate;

    fn has_permission(&self, node: &str) -> bool {
        self.value(node).is_true()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultPermissionChecker;

impl PermissionChecker for DefaultPermissionChecker {
    fn value(&self, _node: &str) -> Tristate {
        Tristate::Undefined
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AllPermissionsChecker;

impl PermissionChecker for AllPermissionsChecker {
    fn value(&self, _node: &str) -> Tristate {
        Tristate::True
    }
}

#[must_use]
pub fn normalize_node(node: &str) -> Cow<'_, str> {
    let node = node.trim();
    if node.chars().any(char::is_uppercase) {
        Cow::Owned(node.to_lowercase())
    } else {
        Cow::Borrowed(node)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionMap {
    nodes: HashMap<String, bool>,
}

impl PermissionMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with(mut self, node: &str, value: bool) -> Self {
        self.set(node, value);
        self
    }

    pub fn set(&mut self, node: &str, value: bool) {
        self.nodes.insert(normalize_node(node).into_owned(), value);
    }

    pub fn unset(&mut self, node: &str) -> Option<bool> {
        self.nodes.remove(normalize_node(node).as_ref())
    }

    #[must_use]
    pub fn get(&self, node: &str) -> Option<bool> {
        self.nodes.get(normalize_node(node).as_ref()).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, bool)> {
        self.nodes
            .iter()
            .map(|(node, value)| (node.as_str(), *value))
    }

    fn lookup(&self, node: &str) -> Option<bool> {
        let node = normalize_node(node);
        if let Some(value) = self.nodes.get(node.as_ref()) {
            return Some(*value);
        }
        let mut end = node.len();
        while let Some(dot) = node[..end].rfind('.') {
            if let Some(value) = self.nodes.get(&format!("{}.{WILDCARD}", &node[..dot])) {
                return Some(*value);
            }
            end = dot;
        }
        self.nodes.get(WILDCARD).copied()
    }
}

impl PermissionChecker for PermissionMap {
    fn value(&self, node: &str) -> Tristate {
        self.lookup(node).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PermissionDefault {
    False,
    True,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PermissionNode {
    pub name: String,
    pub description: String,
    pub default: PermissionDefault,
}

impl PermissionNode {
    pub fn new(name: impl Into<String>, default: PermissionDefault) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            default,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PermissionNodeInfo {
    pub node: PermissionNode,
    pub plugin_id: Option<String>,
}

impl PermissionNodeInfo {
    pub fn new(node: PermissionNode, plugin_id: Option<String>) -> Self {
        Self { node, plugin_id }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PermissionNodeError {
    #[error("'{0}' is not a valid permission node")]
    InvalidName(String),
    #[error("'{0}' is reserved by the proxy")]
    Reserved(String),
    #[error("'{name}' is already registered by plugin '{plugin}'")]
    OwnedBy { name: String, plugin: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionSubject {
    #[non_exhaustive]
    Player {
        player_id: PlayerId,
        profile: GameProfile,
        online_mode: bool,
        virtual_host: Option<String>,
        remote_addr: SocketAddr,
    },
    Console,
}

impl PermissionSubject {
    pub fn player(
        player_id: PlayerId,
        profile: GameProfile,
        online_mode: bool,
        remote_addr: SocketAddr,
    ) -> Self {
        Self::Player {
            player_id,
            profile,
            online_mode,
            virtual_host: None,
            remote_addr,
        }
    }

    #[must_use]
    pub fn with_virtual_host(mut self, host: impl Into<String>) -> Self {
        if let Self::Player { virtual_host, .. } = &mut self {
            *virtual_host = Some(host.into());
        }
        self
    }

    pub fn player_id(&self) -> Option<PlayerId> {
        match self {
            Self::Player { player_id, .. } => Some(*player_id),
            Self::Console => None,
        }
    }

    pub fn profile(&self) -> Option<&GameProfile> {
        match self {
            Self::Player { profile, .. } => Some(profile),
            Self::Console => None,
        }
    }

    pub fn is_online_mode(&self) -> bool {
        matches!(
            self,
            Self::Player {
                online_mode: true,
                ..
            }
        )
    }

    pub fn virtual_host(&self) -> Option<&str> {
        match self {
            Self::Player { virtual_host, .. } => virtual_host.as_deref(),
            Self::Console => None,
        }
    }

    pub fn remote_addr(&self) -> Option<SocketAddr> {
        match self {
            Self::Player { remote_addr, .. } => Some(*remote_addr),
            Self::Console => None,
        }
    }

    pub const fn is_console(&self) -> bool {
        matches!(self, Self::Console)
    }
}

pub trait PermissionProvider: Send + Sync {
    fn create_checker<'a>(
        &'a self,
        subject: &'a PermissionSubject,
    ) -> BoxFuture<'a, Arc<dyn PermissionChecker>>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PermissionProviderRejected {
    #[error("the plugin lacks the permission-provider capability")]
    MissingCapability,
    #[error("[permissions] provider selects `{selected}`, not this plugin")]
    NotSelected { selected: String },
}

/// A capability granted to a plugin.
///
/// The source of truth is the Infrarust config (`[plugins.<id>] permissions = [...]`);
/// compiled-in native plugins are trusted and receive [`CapabilitySet::native_trusted`].
/// This is the plugin-capability model and is unrelated to player permission nodes.
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
    /// Rewrite the global proxy configuration file.
    ConfigWrite,
    /// Register codec filters (WASM: gated; native: implicit).
    CodecFilter,
    TransportFilter,
    /// Provide limbo handlers.
    Limbo,
    /// Provide virtual backends (Tier 3).
    VirtualBackend,
    /// Become the proxy's permission provider (`[permissions] provider`).
    PermissionProvider,
    /// Filesystem access beyond the dedicated `data_dir`.
    FilesystemExtended,
    /// Outbound network access.
    Network,
    ChatIntercept,
    BanProvider,
}

impl Capability {
    pub const ALL: [Capability; 19] = [
        Capability::EventBus,
        Capability::PlayerRead,
        Capability::PlayerWrite,
        Capability::RawPacket,
        Capability::ServerManage,
        Capability::Ban,
        Capability::Command,
        Capability::Scheduler,
        Capability::ConfigRead,
        Capability::ConfigWrite,
        Capability::CodecFilter,
        Capability::TransportFilter,
        Capability::Limbo,
        Capability::VirtualBackend,
        Capability::PermissionProvider,
        Capability::FilesystemExtended,
        Capability::Network,
        Capability::ChatIntercept,
        Capability::BanProvider,
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
            Capability::ConfigWrite => "config-write",
            Capability::CodecFilter => "codec-filter",
            Capability::TransportFilter => "transport-filter",
            Capability::Limbo => "limbo",
            Capability::VirtualBackend => "virtual-backend",
            Capability::PermissionProvider => "permission-provider",
            Capability::FilesystemExtended => "filesystem-extended",
            Capability::Network => "network",
            Capability::ChatIntercept => "chat-intercept",
            Capability::BanProvider => "ban-provider",
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
            "config-write" => Capability::ConfigWrite,
            "codec-filter" => Capability::CodecFilter,
            "transport-filter" => Capability::TransportFilter,
            "limbo" => Capability::Limbo,
            "virtual-backend" => Capability::VirtualBackend,
            "permission-provider" => Capability::PermissionProvider,
            "filesystem-extended" => Capability::FilesystemExtended,
            "network" => Capability::Network,
            "chat-intercept" => Capability::ChatIntercept,
            "ban-provider" => Capability::BanProvider,
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

    pub fn remove(&mut self, cap: Capability) {
        self.granted.remove(&cap);
    }

    #[must_use]
    pub fn with(mut self, cap: Capability) -> Self {
        self.granted.insert(cap);
        self
    }

    #[must_use]
    pub fn without(mut self, cap: Capability) -> Self {
        self.granted.remove(&cap);
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

    #[must_use]
    pub fn revoke_config_strings(&mut self, strings: &[String]) -> Vec<String> {
        let mut unknown = Vec::new();
        for s in strings {
            match Capability::from_kebab(s) {
                Some(cap) => self.remove(cap),
                None => unknown.push(s.clone()),
            }
        }
        unknown
    }

    #[must_use]
    pub fn from_config(grants: &[String], denies: &[String]) -> (Self, Vec<String>) {
        let (mut set, mut rejected) = Self::from_config_strings(grants);
        rejected.extend(set.revoke_config_strings(denies));
        (set, rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_checker_leaves_every_node_undefined() {
        let checker = DefaultPermissionChecker;
        assert_eq!(checker.value(ADMIN_PERMISSION), Tristate::Undefined);
        assert!(!checker.has_permission("anything"));
        assert!(AllPermissionsChecker.has_permission("anything"));
    }

    #[test]
    fn tristate_falls_back_only_when_undefined() {
        assert_eq!(Tristate::Undefined.or(Tristate::True), Tristate::True);
        assert_eq!(Tristate::False.or(Tristate::True), Tristate::False);
        assert_eq!(Tristate::from(Some(true)), Tristate::True);
        assert_eq!(Tristate::from(None), Tristate::Undefined);
        assert_eq!(Tristate::True.as_bool(), Some(true));
        assert_eq!(Tristate::Undefined.as_bool(), None);
    }

    #[test]
    fn a_wildcard_covers_every_child_but_not_its_parent() {
        let map = PermissionMap::new().with("demo.*", true);
        assert_eq!(map.value("demo.use"), Tristate::True);
        assert_eq!(map.value("demo.admin.reload"), Tristate::True);
        assert_eq!(map.value("demo"), Tristate::Undefined);
        assert_eq!(map.value("demolition.use"), Tristate::Undefined);
    }

    #[test]
    fn the_most_specific_entry_wins() {
        let map = PermissionMap::new()
            .with("*", true)
            .with("demo.*", false)
            .with("demo.use", true);
        assert_eq!(map.value("demo.use"), Tristate::True);
        assert_eq!(map.value("demo.kick"), Tristate::False);
        assert_eq!(map.value("other.node"), Tristate::True);
        assert_eq!(map.value("single"), Tristate::True);
    }

    #[test]
    fn nodes_are_case_insensitive() {
        let mut map = PermissionMap::new().with("Demo.Use", true);
        assert_eq!(map.value("DEMO.USE"), Tristate::True);
        assert_eq!(map.get("demo.use"), Some(true));
        assert_eq!(map.unset("demo.USE"), Some(true));
        assert!(map.is_empty());
    }

    #[test]
    fn a_player_subject_carries_its_connection() {
        let profile = GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "Steve".into(),
            properties: vec![],
        };
        let subject = PermissionSubject::player(
            PlayerId::new(7),
            profile.clone(),
            true,
            "127.0.0.1:1".parse().unwrap(),
        )
        .with_virtual_host("play.example.com");
        assert_eq!(subject.player_id(), Some(PlayerId::new(7)));
        assert_eq!(subject.profile(), Some(&profile));
        assert!(subject.is_online_mode());
        assert_eq!(subject.virtual_host(), Some("play.example.com"));
        assert!(!subject.is_console());
        assert!(PermissionSubject::Console.is_console());
        assert_eq!(PermissionSubject::Console.virtual_host(), None);
    }

    #[test]
    fn capability_kebab_roundtrips_for_every_variant() {
        use std::collections::HashSet;
        let mut names = HashSet::new();
        for cap in Capability::ALL {
            let s = cap.to_kebab();
            assert!(names.insert(s), "duplicate kebab name: {s}");
            assert_eq!(
                Capability::from_kebab(s),
                Some(cap),
                "from_kebab lost {cap:?}"
            );
        }
        assert_eq!(Capability::ALL.len(), 19);
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
        assert!(!b.has(Capability::ConfigWrite));
        assert!(!b.has(Capability::CodecFilter));
        assert!(!b.has(Capability::TransportFilter));
        assert!(!b.has(Capability::ChatIntercept));
        assert!(!b.has(Capability::BanProvider));
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

    #[test]
    fn from_config_revokes_denies_after_baseline_and_grants() {
        let (set, rejected) = CapabilitySet::from_config(
            &["ban".to_string(), "limbo".to_string()],
            &[
                "player-write".to_string(),
                "ban".to_string(),
                "not-a-capability".to_string(),
            ],
        );
        assert!(!set.has(Capability::PlayerWrite));
        assert!(!set.has(Capability::Ban));
        assert!(set.has(Capability::Limbo));
        assert!(set.has(Capability::EventBus));
        assert_eq!(rejected, vec!["not-a-capability".to_string()]);
    }

    #[test]
    fn revoke_config_strings_can_strip_a_trusted_set() {
        let mut set = CapabilitySet::native_trusted();
        let unknown = set.revoke_config_strings(&["transport-filter".to_string()]);
        assert!(unknown.is_empty());
        assert!(!set.has(Capability::TransportFilter));
        assert!(set.has(Capability::CodecFilter));
    }
}
