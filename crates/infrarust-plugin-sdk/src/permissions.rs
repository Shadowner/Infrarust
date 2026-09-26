use std::collections::BTreeMap;
use std::net::SocketAddr;

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
    use super::*;

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
}
