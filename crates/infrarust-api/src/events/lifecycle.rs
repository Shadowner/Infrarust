//! Player lifecycle events.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::event::{Event, ResultedEvent};
use crate::permissions::PermissionChecker;
use crate::player::Player;
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

#[non_exhaustive]
pub struct GameProfileRequestEvent {
    pub profile: GameProfile,
    pub online_mode: bool,
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub protocol_version: ProtocolVersion,
    original: GameProfile,
}

impl GameProfileRequestEvent {
    pub fn new(
        profile: GameProfile,
        online_mode: bool,
        remote_addr: SocketAddr,
        virtual_host: Option<String>,
        protocol_version: ProtocolVersion,
    ) -> Self {
        Self {
            original: profile.clone(),
            profile,
            online_mode,
            remote_addr,
            virtual_host,
            protocol_version,
        }
    }

    pub const fn original(&self) -> &GameProfile {
        &self.original
    }

    pub fn is_modified(&self) -> bool {
        self.profile != self.original
    }
}

impl Event for GameProfileRequestEvent {}

/// Fired after a player has successfully authenticated.
///
/// This is informational — the login cannot be cancelled at this point.
#[non_exhaustive]
pub struct PostLoginEvent {
    pub player: Arc<dyn Player>,
    pub profile: GameProfile,
    pub protocol_version: ProtocolVersion,
}

impl PostLoginEvent {
    pub fn new(player: Arc<dyn Player>) -> Self {
        Self {
            profile: player.profile().clone(),
            protocol_version: player.protocol_version(),
            player,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for PostLoginEvent {}

/// Fired when a player disconnects from the proxy.
///
/// This event is **awaited** — the proxy waits for all listeners to finish
/// before cleaning up resources, allowing plugins to do cleanup work.
#[non_exhaustive]
pub struct DisconnectEvent {
    pub player: Arc<dyn Player>,
    pub last_server: Option<ServerId>,
    pub cause: DisconnectCause,
}

impl DisconnectEvent {
    pub fn new(
        player: Arc<dyn Player>,
        last_server: Option<ServerId>,
        cause: DisconnectCause,
    ) -> Self {
        Self {
            player,
            last_server,
            cause,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn username(&self) -> &str {
        &self.player.profile().username
    }
}

impl Event for DisconnectEvent {}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DisconnectCause {
    ClientQuit,
    Kicked { reason: Option<Component> },
    BackendClosed { reason: Option<Component> },
    Shutdown,
    Error,
}

impl DisconnectCause {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ClientQuit => "client_quit",
            Self::Kicked { .. } => "kicked",
            Self::BackendClosed { .. } => "backend_closed",
            Self::Shutdown => "shutdown",
            Self::Error => "error",
        }
    }

    pub const fn reason(&self) -> Option<&Component> {
        match self {
            Self::Kicked { reason } | Self::BackendClosed { reason } => reason.as_ref(),
            Self::ClientQuit | Self::Shutdown | Self::Error => None,
        }
    }
}

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
    pub player: Arc<dyn Player>,
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
    pub fn new(player: Arc<dyn Player>, online_mode: bool) -> Self {
        Self {
            player,
            online_mode,
            result: PermissionsSetupResult::default(),
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
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

pub struct LoginEvent {
    pub player: Arc<dyn Player>,
    pub online_mode: bool,
    result: LoginResult,
}

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum LoginResult {
    #[default]
    Allowed,
    Denied {
        reason: Component,
    },
}

impl LoginEvent {
    pub fn new(player: Arc<dyn Player>, online_mode: bool) -> Self {
        Self {
            player,
            online_mode,
            result: LoginResult::default(),
        }
    }

    pub fn deny(&mut self, reason: Component) {
        self.result = LoginResult::Denied { reason };
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
    }
}

impl Event for LoginEvent {}
impl ResultedEvent for LoginEvent {
    type Result = LoginResult;

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
    fn game_profile_request_keeps_the_original_profile() {
        let mut event = GameProfileRequestEvent::new(
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            false,
            "127.0.0.1:25565".parse().unwrap(),
            Some("play.example.com".into()),
            ProtocolVersion::MINECRAFT_1_21,
        );
        assert!(!event.is_modified());

        event.profile.username = "Alex".into();

        assert!(event.is_modified());
        assert_eq!(event.original().username, "Steve");
        assert_eq!(event.profile.username, "Alex");
    }

    #[test]
    fn disconnect_cause_names_and_reasons() {
        let kicked = DisconnectCause::Kicked {
            reason: Some(Component::text("bye")),
        };
        assert_eq!(kicked.as_str(), "kicked");
        assert_eq!(kicked.reason(), Some(&Component::text("bye")));
        assert_eq!(DisconnectCause::ClientQuit.as_str(), "client_quit");
        assert_eq!(DisconnectCause::ClientQuit.reason(), None);
        assert_eq!(
            DisconnectCause::BackendClosed { reason: None }.as_str(),
            "backend_closed"
        );
        assert_eq!(DisconnectCause::Shutdown.as_str(), "shutdown");
        assert_eq!(DisconnectCause::Error.as_str(), "error");
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
