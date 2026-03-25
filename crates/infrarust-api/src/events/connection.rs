//! Connection and server routing events.

use crate::event::{Event, ResultedEvent};
use crate::types::{Component, GameProfile, PlayerId, ServerId};
use crate::virtual_backend::VirtualBackendHandler;

/// Fired before the proxy connects a player to a backend server.
///
/// Listeners can redirect the player to a different server, send them
/// to a limbo handler, route them to a virtual backend, or deny the
/// connection entirely.
pub struct ServerPreConnectEvent {
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The player's game profile.
    pub profile: GameProfile,
    /// The server the player was originally going to connect to.
    pub original_server: ServerId,
    result: ServerPreConnectResult,
}

impl ServerPreConnectEvent {
    pub fn new(player_id: PlayerId, profile: GameProfile, original_server: ServerId) -> Self {
        Self {
            player_id,
            profile,
            original_server,
            result: ServerPreConnectResult::default(),
        }
    }

    /// Shortcut: redirect to a different server.
    pub fn redirect_to(&mut self, server: ServerId) {
        self.result = ServerPreConnectResult::ConnectTo(server);
    }

    /// Shortcut: deny the connection with a reason.
    pub fn deny(&mut self, reason: Component) {
        self.result = ServerPreConnectResult::Denied { reason };
    }
}

/// The result of a [`ServerPreConnectEvent`].
#[derive(Default)]
#[non_exhaustive]
pub enum ServerPreConnectResult {
    /// Allow the connection to the original server.
    #[default]
    Allowed,
    /// Redirect to a different backend server.
    ConnectTo(ServerId),
    /// Send the player to the limbo handler chain.
    SendToLimbo { limbo_handlers: Vec<String> },
    /// Route the player to a virtual backend handler.
    VirtualBackend(Box<dyn VirtualBackendHandler>),
    /// Deny the connection with a reason.
    Denied {
        /// The reason shown to the player.
        reason: Component,
    },
}

impl Event for ServerPreConnectEvent {}
impl ResultedEvent for ServerPreConnectEvent {
    type Result = ServerPreConnectResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

/// Fired after a player has successfully connected to a backend server.
///
/// Informational — the connection is already established.
pub struct ServerConnectedEvent {
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The server the player connected to.
    pub server: ServerId,
}

impl Event for ServerConnectedEvent {}

/// Fired after a player switches from one server to another.
///
/// Informational — the switch has already completed.
pub struct ServerSwitchEvent {
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The server the player was previously on.
    pub previous_server: ServerId,
    /// The server the player is now on.
    pub new_server: ServerId,
}

impl Event for ServerSwitchEvent {}

/// Fired when a backend server kicks a player.
///
/// Listeners can decide whether to disconnect the player, redirect them
/// to another server, send them to limbo, or just notify them.
pub struct KickedFromServerEvent {
    /// The player's session ID.
    pub player_id: PlayerId,
    /// The server that kicked the player.
    pub server: ServerId,
    /// The kick reason from the backend server.
    pub reason: Component,
    result: KickedFromServerResult,
}

impl KickedFromServerEvent {
    pub fn new(player_id: PlayerId, server: ServerId, reason: Component) -> Self {
        Self {
            player_id,
            server,
            reason,
            result: KickedFromServerResult::default(),
        }
    }

    /// Shortcut: redirect the player to another server.
    pub fn redirect_to(&mut self, server: ServerId) {
        self.result = KickedFromServerResult::RedirectTo(server);
    }
}

/// The result of a [`KickedFromServerEvent`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum KickedFromServerResult {
    /// Disconnect the player from the proxy entirely.
    DisconnectPlayer {
        /// The reason shown to the player.
        reason: Component,
    },
    /// Redirect the player to a different server.
    RedirectTo(ServerId),
    /// Send the player to the limbo handler chain.
    SendToLimbo { limbo_handlers: Vec<String> },
    /// Keep the player on the proxy but notify them of the kick.
    Notify {
        /// A message shown to the player.
        message: Component,
    },
}

impl Default for KickedFromServerResult {
    fn default() -> Self {
        Self::DisconnectPlayer {
            reason: Component::error("Kicked from server"),
        }
    }
}

impl Event for KickedFromServerEvent {}
impl ResultedEvent for KickedFromServerEvent {
    type Result = KickedFromServerResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

/// Dispatched after PostLoginEvent, before ServerPreConnectEvent.
/// Allows a plugin to redirect the player to a different server
/// than the one resolved by the DomainRouter.
///
/// Use cases: lobby plugin, load balancer, queue system.
pub struct PlayerChooseInitialServerEvent {
    /// The player connecting.
    pub player_id: PlayerId,
    /// The player's game profile.
    pub profile: GameProfile,
    /// The server resolved by the DomainRouter (default target).
    pub initial_server: ServerId,
    result: PlayerChooseInitialServerResult,
}

/// Result of a [`PlayerChooseInitialServerEvent`].
#[derive(Default, Clone)]
#[non_exhaustive]
pub enum PlayerChooseInitialServerResult {
    /// Use the server resolved by the DomainRouter.
    #[default]
    Allowed,
    /// Redirect to a different server.
    Redirect(ServerId),
    SendToLimbo {
        limbo_handlers: Vec<String>,
    },
}

impl PlayerChooseInitialServerEvent {
    pub fn new(player_id: PlayerId, profile: GameProfile, initial_server: ServerId) -> Self {
        Self {
            player_id,
            profile,
            initial_server,
            result: PlayerChooseInitialServerResult::default(),
        }
    }

    pub fn result(&self) -> &PlayerChooseInitialServerResult {
        &self.result
    }

    pub fn set_result(&mut self, result: PlayerChooseInitialServerResult) {
        self.result = result;
    }
}

impl Event for PlayerChooseInitialServerEvent {}
impl ResultedEvent for PlayerChooseInitialServerEvent {
    type Result = PlayerChooseInitialServerResult;

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
    use crate::types::GameProfile;

    #[test]
    fn server_pre_connect_default() {
        let event = ServerPreConnectEvent::new(
            PlayerId::new(1),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            ServerId::new("lobby"),
        );
        assert!(matches!(event.result(), ServerPreConnectResult::Allowed));
    }

    #[test]
    fn server_pre_connect_redirect() {
        let mut event = ServerPreConnectEvent::new(
            PlayerId::new(1),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".into(),
                properties: vec![],
            },
            ServerId::new("lobby"),
        );
        event.redirect_to(ServerId::new("survival"));
        assert!(matches!(
            event.result(),
            ServerPreConnectResult::ConnectTo(_)
        ));
    }

    #[test]
    fn kicked_default_disconnects() {
        let event = KickedFromServerEvent::new(
            PlayerId::new(1),
            ServerId::new("lobby"),
            Component::text("Banned"),
        );
        assert!(matches!(
            event.result(),
            KickedFromServerResult::DisconnectPlayer { .. }
        ));
    }

    #[test]
    fn kicked_redirect() {
        let mut event = KickedFromServerEvent::new(
            PlayerId::new(1),
            ServerId::new("lobby"),
            Component::text("Restarting"),
        );
        event.redirect_to(ServerId::new("hub"));
        assert!(matches!(
            event.result(),
            KickedFromServerResult::RedirectTo(_)
        ));
    }

    #[test]
    fn non_exhaustive_kicked_result() {
        let result = KickedFromServerResult::SendToLimbo {
            limbo_handlers: vec![],
        };
        #[allow(unreachable_patterns)]
        match result {
            KickedFromServerResult::DisconnectPlayer { .. }
            | KickedFromServerResult::RedirectTo(_)
            | KickedFromServerResult::SendToLimbo { .. }
            | KickedFromServerResult::Notify { .. }
            | _ => {}
        }
    }
}
