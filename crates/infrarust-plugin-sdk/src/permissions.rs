use std::collections::BTreeMap;
use std::net::SocketAddr;

use crate::bindings::permission_nodes as wpn;
use crate::bindings::permissions as wp;
use crate::error::Error;
use crate::types::{GameProfile, PlayerId, socket_from_wit};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionSnapshot {
    rules: BTreeMap<String, bool>,
    admin: bool,
}

fn normalize(node: &str) -> String {
    node.trim().to_lowercase()
}

impl PermissionSnapshot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn admin() -> Self {
        Self {
            rules: BTreeMap::new(),
            admin: true,
        }
    }

    #[must_use]
    pub fn with(mut self, node: &str, value: bool) -> Self {
        self.set(node, value);
        self
    }

    #[must_use]
    pub fn grant(self, node: &str) -> Self {
        self.with(node, true)
    }

    #[must_use]
    pub fn deny(self, node: &str) -> Self {
        self.with(node, false)
    }

    pub fn set(&mut self, node: &str, value: bool) {
        self.rules.insert(normalize(node), value);
    }

    pub fn unset(&mut self, node: &str) -> Option<bool> {
        self.rules.remove(&normalize(node))
    }

    #[must_use]
    pub fn get(&self, node: &str) -> Option<bool> {
        self.rules.get(&normalize(node)).copied()
    }

    pub fn set_admin(&mut self, admin: bool) {
        self.admin = admin;
    }

    #[must_use]
    pub const fn is_admin(&self) -> bool {
        self.admin
    }

    pub fn rules(&self) -> impl Iterator<Item = (&str, bool)> {
        self.rules
            .iter()
            .map(|(node, value)| (node.as_str(), *value))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub(crate) fn to_wit(&self) -> wp::PermissionSnapshot {
        wp::PermissionSnapshot {
            rules: self
                .rules
                .iter()
                .map(|(node, value)| wp::PermissionRule {
                    node: node.clone(),
                    value: *value,
                })
                .collect(),
            admin: self.admin,
        }
    }

    pub(crate) fn from_wit(snapshot: wp::PermissionSnapshot) -> Self {
        let mut native = Self::new();
        native.admin = snapshot.admin;
        for rule in snapshot.rules {
            native.set(&rule.node, rule.value);
        }
        native
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerSubject {
    pub id: PlayerId,
    pub profile: GameProfile,
    pub online_mode: bool,
    pub virtual_host: Option<String>,
    pub remote_addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PermissionSubject {
    Player(PlayerSubject),
    Console,
}

impl PermissionSubject {
    #[must_use]
    pub const fn player_id(&self) -> Option<PlayerId> {
        match self {
            Self::Player(player) => Some(player.id),
            Self::Console => None,
        }
    }

    #[must_use]
    pub const fn profile(&self) -> Option<&GameProfile> {
        match self {
            Self::Player(player) => Some(&player.profile),
            Self::Console => None,
        }
    }

    #[must_use]
    pub const fn is_console(&self) -> bool {
        matches!(self, Self::Console)
    }

    pub(crate) fn from_wit(subject: wp::PermissionSubject) -> Self {
        match subject {
            wp::PermissionSubject::Player(player) => Self::Player(PlayerSubject {
                id: PlayerId::new(player.id),
                profile: GameProfile::from_wit(player.profile),
                online_mode: player.online_mode,
                virtual_host: player.virtual_host,
                remote_addr: socket_from_wit(player.remote_addr),
            }),
            wp::PermissionSubject::Console => Self::Console,
        }
    }
}

pub trait PermissionProvider {
    fn snapshot_for(&self, subject: &PermissionSubject) -> PermissionSnapshot;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PermissionDefault {
    False,
    True,
    Admin,
}

impl PermissionDefault {
    pub(crate) const fn to_wit(self) -> wpn::PermissionDefault {
        match self {
            Self::False => wpn::PermissionDefault::False,
            Self::True => wpn::PermissionDefault::True,
            Self::Admin => wpn::PermissionDefault::Admin,
        }
    }

    pub(crate) const fn from_wit(default: wpn::PermissionDefault) -> Self {
        match default {
            wpn::PermissionDefault::False => Self::False,
            wpn::PermissionDefault::True => Self::True,
            wpn::PermissionDefault::Admin => Self::Admin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PermissionNode {
    pub name: String,
    pub description: String,
    pub default: PermissionDefault,
}

impl PermissionNode {
    #[must_use]
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

    pub(crate) fn into_wit(self) -> wpn::PermissionNode {
        wpn::PermissionNode {
            name: self.name,
            description: self.description,
            default: self.default.to_wit(),
        }
    }

    pub(crate) fn from_wit(node: wpn::PermissionNode) -> Self {
        Self {
            name: node.name,
            description: node.description,
            default: PermissionDefault::from_wit(node.default),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PermissionNodeInfo {
    pub node: PermissionNode,
    pub plugin_id: Option<String>,
}

impl PermissionNodeInfo {
    pub(crate) fn from_wit(info: wpn::PermissionNodeInfo) -> Self {
        Self {
            node: PermissionNode::from_wit(info.node),
            plugin_id: info.plugin_id,
        }
    }
}

pub struct Permissions;

impl Permissions {
    pub fn set_snapshot(player: PlayerId, snapshot: &PermissionSnapshot) -> Result<(), Error> {
        Ok(crate::host::set_snapshot(
            player.as_u64(),
            &snapshot.to_wit(),
        )?)
    }

    pub fn release(player: PlayerId) -> Result<(), Error> {
        Ok(crate::host::release_snapshot(player.as_u64())?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::context::Context;
    use crate::error::ErrorKind;

    #[test]
    fn a_snapshot_normalizes_its_nodes_and_crosses_the_boundary_intact() {
        let snapshot = PermissionSnapshot::new()
            .grant(" Demo.Use ")
            .deny("demo.*")
            .with("other", true);
        assert_eq!(snapshot.get("DEMO.USE"), Some(true));
        assert_eq!(snapshot.len(), 3);
        let wit = snapshot.to_wit();
        assert_eq!(
            wit.rules
                .iter()
                .map(|rule| (rule.node.as_str(), rule.value))
                .collect::<Vec<_>>(),
            [("demo.*", false), ("demo.use", true), ("other", true)]
        );
        assert_eq!(PermissionSnapshot::from_wit(wit), snapshot);
        assert!(PermissionSnapshot::admin().is_admin());
        assert!(PermissionSnapshot::admin().is_empty());
    }

    #[test]
    fn a_player_subject_carries_its_connection() {
        let subject =
            PermissionSubject::from_wit(wp::PermissionSubject::Player(wp::PlayerSubject {
                id: 7,
                profile: crate::bindings::types::GameProfile {
                    uuid: crate::bindings::types::Uuid { hi: 0, lo: 1 },
                    username: "Steve".into(),
                    properties: vec![],
                },
                online_mode: true,
                virtual_host: Some("play.example.com".into()),
                remote_addr: crate::bindings::types::SocketAddress {
                    ip: crate::bindings::types::IpAddress::Ipv4((203, 0, 113, 7)),
                    port: 51234,
                },
            }));
        assert_eq!(subject.player_id(), Some(PlayerId::new(7)));
        assert_eq!(
            subject.profile().map(|p| p.username.as_str()),
            Some("Steve")
        );
        let PermissionSubject::Player(player) = &subject else {
            panic!("a player subject");
        };
        assert_eq!(player.remote_addr, "203.0.113.7:51234".parse().unwrap());
        assert!(PermissionSubject::from_wit(wp::PermissionSubject::Console).is_console());
    }

    fn registered(name: &str) -> Option<wpn::PermissionNodeInfo> {
        crate::host::with_fake(|host| host.permission_nodes.get(name).cloned())
    }

    #[test]
    fn a_registered_node_reaches_the_host_with_its_default_and_description() {
        let ctx = Context::new();
        ctx.register_permission_node(
            PermissionNode::new("Warps.Use", PermissionDefault::Admin).description("Use warps"),
        )
        .unwrap();
        let held = registered("warps.use").expect("the host holds the node");
        assert_eq!(held.node.default, wpn::PermissionDefault::Admin);
        assert_eq!(held.node.description, "Use warps");

        ctx.register_permission_node(PermissionNode::new("warps.use", PermissionDefault::True))
            .unwrap();
        let replaced = registered("warps.use").unwrap();
        assert_eq!(replaced.node.default, wpn::PermissionDefault::True);
        assert_eq!(replaced.node.description, "");
    }

    #[test]
    fn a_refused_node_answers_the_hosts_error() {
        crate::host::with_fake(|host| host.refused.insert("warps.admin".to_owned()));
        let refused = Context::new()
            .register_permission_node(PermissionNode::new("warps.admin", PermissionDefault::False))
            .unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::Conflict);
        assert_eq!(refused.message(), "warps.admin is refused");
        assert_eq!(registered("warps.admin"), None);
    }

    #[test]
    fn nodes_are_read_back_with_their_owner() {
        crate::host::with_fake(|host| {
            host.permission_nodes.insert(
                "infrarust.admin".to_owned(),
                wpn::PermissionNodeInfo {
                    node: wpn::PermissionNode {
                        name: "infrarust.admin".to_owned(),
                        description: "Every proxy command".to_owned(),
                        default: wpn::PermissionDefault::False,
                    },
                    plugin_id: None,
                },
            );
        });
        let ctx = Context::new();
        ctx.register_permission_node(PermissionNode::new("warps.use", PermissionDefault::True))
            .unwrap();

        let warps = ctx.permission_node(" WARPS.Use ").unwrap();
        assert_eq!(
            warps.node,
            PermissionNode::new("warps.use", PermissionDefault::True)
        );
        assert_eq!(warps.plugin_id.as_deref(), Some("fake"));
        let admin = ctx.permission_node("infrarust.admin").unwrap();
        assert_eq!(admin.plugin_id, None);
        assert_eq!(admin.node.default, PermissionDefault::False);
        assert_eq!(admin.node.description, "Every proxy command");
        assert_eq!(ctx.permission_node("nobody.registered"), None);
        assert_eq!(
            ctx.permission_nodes()
                .into_iter()
                .map(|info| (info.node.name, info.plugin_id))
                .collect::<Vec<_>>(),
            [
                ("infrarust.admin".to_owned(), None),
                ("warps.use".to_owned(), Some("fake".to_owned())),
            ]
        );
    }

    #[test]
    fn every_default_crosses_the_boundary_unchanged() {
        for default in [
            PermissionDefault::False,
            PermissionDefault::True,
            PermissionDefault::Admin,
        ] {
            assert_eq!(PermissionDefault::from_wit(default.to_wit()), default);
        }
        let node = PermissionNode::new("warps.fly", PermissionDefault::Admin).description("Fly");
        assert_eq!(PermissionNode::from_wit(node.clone().into_wit()), node);
    }
}
