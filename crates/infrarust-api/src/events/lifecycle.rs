//! Player lifecycle events.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::event::{Event, ResultedEvent};
use crate::permissions::PermissionChecker;
use crate::types::{Component, GameProfile, PlayerId, ProtocolVersion, ServerId};

/// Fired before authentication, when a player initiates a connection.
///
/// Listeners can deny the connection, force offline/online mode, or
/// let it proceed normally.
pub struct PreLoginEvent {
    /// The player's game profile (may be incomplete in offline mode).
    pub profile: GameProfile,
    /// The remote address of the connecting client.
    pub remote_addr: SocketAddr,
    /// The protocol version reported by the client.
    pub protocol_version: ProtocolVersion,
    /// The server domain the client connected to (from the handshake).
    pub server_domain: String,
    result: PreLoginResult,
}

impl PreLoginEvent {
    pub fn new(
        profile: GameProfile,
        remote_addr: SocketAddr,
        protocol_version: ProtocolVersion,
        server_domain: String,
    ) -> Self {
        Self {
            profile,
            remote_addr,
            protocol_version,
            server_domain,
            result: PreLoginResult::default(),
        }
    }

    /// Shortcut: deny the login with a reason message.
    pub fn deny(&mut self, reason: Component) {
        self.result = PreLoginResult::Denied { reason };
    }
}

/// The result of a [`PreLoginEvent`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum PreLoginResult {
    /// Allow the login to proceed normally.
    #[default]
    Allowed,
    /// Deny the login with a kick reason.
    Denied {
        /// The reason shown to the player.
        reason: Component,
    },
    /// Force offline-mode authentication for this player.
    ForceOffline,
    /// Force online-mode authentication for this player.
    ForceOnline,
}

impl Event for PreLoginEvent {}
impl ResultedEvent for PreLoginEvent {
    type Result = PreLoginResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

/// Fired after a player has successfully authenticated.
///
/// This is informational — the login cannot be cancelled at this point.
pub struct PostLoginEvent {
    /// The authenticated player's game profile.
    pub profile: GameProfile,
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The player's protocol version.
    pub protocol_version: ProtocolVersion,
}

impl Event for PostLoginEvent {}

/// Fired when a player disconnects from the proxy.
///
/// This event is **awaited** — the proxy waits for all listeners to finish
/// before cleaning up resources, allowing plugins to do cleanup work.
pub struct DisconnectEvent {
    /// The disconnecting player's ID.
    pub player_id: PlayerId,
    /// The player's username (for logging convenience).
    pub username: String,
    /// The server the player was connected to, if any.
    pub last_server: Option<ServerId>,
}

impl Event for DisconnectEvent {}

/// Fired when online-mode authentication fails (cracked client couldn't complete encryption).
///
/// Covers both forced online auth (`ForceOnline` in offline mode) and default
/// online auth (`client_only` mode). Plugins can listen for this to remember
/// the username and set `ForceOffline` on the next connection attempt.
pub struct OnlineAuthFailed {
    /// The username that failed online authentication.
    pub username: String,
}

impl Event for OnlineAuthFailed {}

/// Fired after authentication, before the player session is fully constructed.
///
/// Plugins can listen for this event to provide a custom [`PermissionChecker`]
/// that replaces the default config-based checker for this player. This is the
/// extension point for integrating LuckPerms, a database, or any external
/// permission system.
///
/// If no listener sets a custom checker, the proxy uses its built-in
/// `ConfigPermissionChecker` (admin UUIDs from `[permissions].admins`).
pub struct PermissionsSetupEvent {
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The authenticated game profile.
    pub profile: GameProfile,
    /// Whether the player authenticated via Mojang (online mode).
    pub online_mode: bool,
    result: PermissionsSetupResult,
}

/// The result of a [`PermissionsSetupEvent`].
#[derive(Default)]
#[non_exhaustive]
pub enum PermissionsSetupResult {
    /// Use the proxy's built-in config-based permission checker.
    #[default]
    UseDefault,
    /// Use a plugin-provided permission checker.
    Custom(Arc<dyn PermissionChecker>),
}

impl PermissionsSetupEvent {
    pub fn new(player_id: PlayerId, profile: GameProfile, online_mode: bool) -> Self {
        Self {
            player_id,
            profile,
            online_mode,
            result: PermissionsSetupResult::default(),
        }
    }
}

impl Event for PermissionsSetupEvent {}
impl ResultedEvent for PermissionsSetupEvent {
    type Result = PermissionsSetupResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn pre_login_default_result() {
        let event = PreLoginEvent::new(
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            "127.0.0.1:25565".parse().unwrap(),
            ProtocolVersion::MINECRAFT_1_21,
            "play.example.com".into(),
        );
        assert!(matches!(event.result(), PreLoginResult::Allowed));
    }

    #[test]
    fn pre_login_deny() {
        let mut event = PreLoginEvent::new(
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            "127.0.0.1:25565".parse().unwrap(),
            ProtocolVersion::MINECRAFT_1_21,
            "play.example.com".into(),
        );

        event.deny(Component::error("Banned"));
        assert!(matches!(event.result(), PreLoginResult::Denied { .. }));
    }

    #[test]
    fn pre_login_set_result() {
        let mut event = PreLoginEvent::new(
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            "127.0.0.1:25565".parse().unwrap(),
            ProtocolVersion::MINECRAFT_1_21,
            "play.example.com".into(),
        );

        event.set_result(PreLoginResult::ForceOffline);
        assert!(matches!(event.result(), PreLoginResult::ForceOffline));
    }

    #[test]
    fn non_exhaustive_result_match() {
        let result = PreLoginResult::Allowed;
        #[allow(unreachable_patterns)]
        match result {
            PreLoginResult::Allowed
            | PreLoginResult::Denied { .. }
            | PreLoginResult::ForceOffline
            | PreLoginResult::ForceOnline
            | _ => {}
        }
    }
}
