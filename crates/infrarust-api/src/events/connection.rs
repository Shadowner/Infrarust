//! Connection and server routing events.

use std::sync::Arc;

use crate::event::{Event, ResultedEvent};
use crate::player::Player;
use crate::types::{Component, GameProfile, PlayerId, ServerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectCause {
    Initial,
    Switch,
    LimboExit,
    KickRedirect,
}

impl ConnectCause {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Switch => "switch",
            Self::LimboExit => "limbo_exit",
            Self::KickRedirect => "kick_redirect",
        }
    }
}

/// Fired before the proxy connects a player to a backend server.
///
/// Listeners can redirect the player to a different server, send them
/// to a limbo handler, or deny the connection entirely.
pub struct ServerPreConnectEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
    pub cause: ConnectCause,
    result: ServerPreConnectResult,
}

impl ServerPreConnectEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        previous_server: Option<ServerId>,
        cause: ConnectCause,
    ) -> Self {
        Self {
            player,
            server,
            previous_server,
            cause,
            result: ServerPreConnectResult::default(),
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
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
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum ServerPreConnectResult {
    /// Allow the connection to the original server.
    #[default]
    Allowed,
    /// Redirect to a different backend server.
    ConnectTo(ServerId),
    /// Send the player to the limbo handler chain.
    SendToLimbo { limbo_handlers: Vec<String> },
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

#[non_exhaustive]
pub struct ServerConnectedEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
}

impl ServerConnectedEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        previous_server: Option<ServerId>,
    ) -> Self {
        Self {
            player,
            server,
            previous_server,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for ServerConnectedEvent {}

#[non_exhaustive]
pub struct ServerPostConnectEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
}

impl ServerPostConnectEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        previous_server: Option<ServerId>,
    ) -> Self {
        Self {
            player,
            server,
            previous_server,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn switched_from(&self) -> Option<&ServerId> {
        self.previous_server
            .as_ref()
            .filter(|previous| **previous != self.server)
    }
}

impl Event for ServerPostConnectEvent {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KickCause {
    Unreachable { error: String },
    LoginRefused,
    ConfigDisconnect,
    PlayDisconnect,
    ConnectionLost,
}

impl KickCause {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unreachable { .. } => "unreachable",
            Self::LoginRefused => "login_refused",
            Self::ConfigDisconnect => "config_disconnect",
            Self::PlayDisconnect => "play_disconnect",
            Self::ConnectionLost => "connection_lost",
        }
    }
}

#[non_exhaustive]
pub struct KickedFromServerEvent {
    pub player: Arc<dyn Player>,
    pub server: ServerId,
    pub reason: Option<Component>,
    pub cause: KickCause,
    pub during_connect: bool,
    pub previous_server: Option<ServerId>,
    result: KickedFromServerResult,
}

impl KickedFromServerEvent {
    pub fn new(
        player: Arc<dyn Player>,
        server: ServerId,
        reason: Option<Component>,
        cause: KickCause,
        during_connect: bool,
        previous_server: Option<ServerId>,
        result: KickedFromServerResult,
    ) -> Self {
        Self {
            player,
            server,
            reason,
            cause,
            during_connect,
            previous_server,
            result,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
    }

    pub fn redirect_to(&mut self, server: ServerId) {
        self.result = KickedFromServerResult::RedirectTo(server);
    }

    pub fn disconnect(&mut self, reason: Component) {
        self.result = KickedFromServerResult::DisconnectPlayer {
            reason: Some(reason),
        };
    }

    pub fn send_to_limbo(&mut self, limbo_handlers: Vec<String>) {
        self.result = KickedFromServerResult::SendToLimbo { limbo_handlers };
    }

    pub fn notify(&mut self, message: Component) {
        self.result = KickedFromServerResult::Notify { message };
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum KickedFromServerResult {
    DisconnectPlayer { reason: Option<Component> },
    RedirectTo(ServerId),
    SendToLimbo { limbo_handlers: Vec<String> },
    Notify { message: Component },
}

impl Default for KickedFromServerResult {
    fn default() -> Self {
        Self::DisconnectPlayer { reason: None }
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
    pub player: Arc<dyn Player>,
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
    pub fn new(player: Arc<dyn Player>, initial_server: ServerId) -> Self {
        Self {
            player,
            initial_server,
            result: PlayerChooseInitialServerResult::default(),
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
    }

    /// Shortcut: redirect the player to a different server.
    pub fn redirect_to(&mut self, server: ServerId) {
        self.result = PlayerChooseInitialServerResult::Redirect(server);
    }

    /// Shortcut: send the player to the limbo handler chain.
    pub fn send_to_limbo(&mut self, limbo_handlers: Vec<String>) {
        self.result = PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers };
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
    use std::net::SocketAddr;
    use std::time::SystemTime;

    use super::*;
    use crate::error::PlayerError;
    use crate::event::BoxFuture;
    use crate::permissions::PermissionLevel;
    use crate::types::{ProtocolVersion, RawPacket, TitleData};

    struct Steve(GameProfile);

    impl crate::player::private::Sealed for Steve {}

    impl Player for Steve {
        fn id(&self) -> PlayerId {
            PlayerId::new(1)
        }
        fn profile(&self) -> &GameProfile {
            &self.0
        }
        fn protocol_version(&self) -> ProtocolVersion {
            ProtocolVersion::MINECRAFT_1_21
        }
        fn remote_addr(&self) -> SocketAddr {
            SocketAddr::from(([127, 0, 0, 1], 25565))
        }
        fn current_server(&self) -> Option<ServerId> {
            None
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn is_active(&self) -> bool {
            true
        }
        fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn send_message(&self, _message: Component) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_title(&self, _title: TitleData) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_action_bar(&self, _message: Component) -> Result<(), PlayerError> {
            Ok(())
        }
        fn send_packet(&self, _packet: RawPacket) -> Result<(), PlayerError> {
            Ok(())
        }
        fn switch_server(&self, _target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
            Box::pin(async { Ok(()) })
        }
        fn is_online_mode(&self) -> bool {
            false
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Player
        }
        fn has_permission(&self, _permission: &str) -> bool {
            false
        }
        fn connected_at(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }
    }

    fn steve() -> Arc<dyn Player> {
        Arc::new(Steve(GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "Steve".into(),
            properties: vec![],
        }))
    }

    fn pre_connect() -> ServerPreConnectEvent {
        ServerPreConnectEvent::new(steve(), ServerId::new("lobby"), None, ConnectCause::Initial)
    }

    #[test]
    fn server_pre_connect_default() {
        let event = pre_connect();
        assert!(matches!(event.result(), ServerPreConnectResult::Allowed));
        assert_eq!(event.player_id(), PlayerId::new(1));
        assert_eq!(event.profile().username, "Steve");
    }

    #[test]
    fn server_pre_connect_redirect() {
        let mut event = pre_connect();
        event.redirect_to(ServerId::new("survival"));
        assert!(matches!(
            event.result(),
            ServerPreConnectResult::ConnectTo(_)
        ));
    }

    #[test]
    fn connect_cause_names() {
        assert_eq!(ConnectCause::Initial.as_str(), "initial");
        assert_eq!(ConnectCause::Switch.as_str(), "switch");
        assert_eq!(ConnectCause::LimboExit.as_str(), "limbo_exit");
        assert_eq!(ConnectCause::KickRedirect.as_str(), "kick_redirect");
    }

    #[test]
    fn a_post_connect_is_a_switch_only_when_the_server_changed() {
        let lobby = ServerId::new("lobby");
        let survival = ServerId::new("survival");
        let first = ServerPostConnectEvent::new(steve(), lobby.clone(), None);
        let back = ServerPostConnectEvent::new(steve(), lobby.clone(), Some(lobby.clone()));
        let moved = ServerPostConnectEvent::new(steve(), survival, Some(lobby.clone()));

        assert_eq!(first.switched_from(), None);
        assert_eq!(back.switched_from(), None);
        assert_eq!(moved.switched_from(), Some(&lobby));
    }

    fn kicked(result: KickedFromServerResult) -> KickedFromServerEvent {
        KickedFromServerEvent::new(
            steve(),
            ServerId::new("lobby"),
            Some(Component::text("Banned")),
            KickCause::PlayDisconnect,
            false,
            None,
            result,
        )
    }

    #[test]
    fn kicked_keeps_the_default_the_proxy_chose() {
        let event = kicked(KickedFromServerResult::Notify {
            message: Component::text("Banned"),
        });
        assert_eq!(
            event.result(),
            &KickedFromServerResult::Notify {
                message: Component::text("Banned")
            }
        );
        assert_eq!(event.player_id(), PlayerId::new(1));
        assert_eq!(event.profile().username, "Steve");
        assert_eq!(
            KickedFromServerResult::default(),
            KickedFromServerResult::DisconnectPlayer { reason: None }
        );
    }

    #[test]
    fn kicked_shortcuts() {
        let mut event = kicked(KickedFromServerResult::default());
        event.redirect_to(ServerId::new("hub"));
        assert_eq!(
            event.result(),
            &KickedFromServerResult::RedirectTo(ServerId::new("hub"))
        );
        event.disconnect(Component::text("bye"));
        assert_eq!(
            event.result(),
            &KickedFromServerResult::DisconnectPlayer {
                reason: Some(Component::text("bye"))
            }
        );
        event.send_to_limbo(vec!["queue".into()]);
        assert_eq!(
            event.result(),
            &KickedFromServerResult::SendToLimbo {
                limbo_handlers: vec!["queue".into()]
            }
        );
        event.notify(Component::text("moved"));
        assert_eq!(
            event.result(),
            &KickedFromServerResult::Notify {
                message: Component::text("moved")
            }
        );
    }

    #[test]
    fn kick_cause_names() {
        let unreachable = KickCause::Unreachable {
            error: "refused".into(),
        };
        assert_eq!(unreachable.as_str(), "unreachable");
        assert_eq!(KickCause::LoginRefused.as_str(), "login_refused");
        assert_eq!(KickCause::ConfigDisconnect.as_str(), "config_disconnect");
        assert_eq!(KickCause::PlayDisconnect.as_str(), "play_disconnect");
        assert_eq!(KickCause::ConnectionLost.as_str(), "connection_lost");
    }

    fn choose_initial_event() -> PlayerChooseInitialServerEvent {
        PlayerChooseInitialServerEvent::new(steve(), ServerId::new("lobby"))
    }

    #[test]
    fn choose_initial_default_allowed() {
        let event = choose_initial_event();
        assert!(matches!(
            event.result(),
            PlayerChooseInitialServerResult::Allowed
        ));
        assert_eq!(event.profile().username, "Steve");
    }

    #[test]
    fn choose_initial_redirect_shortcut() {
        let mut event = choose_initial_event();
        event.redirect_to(ServerId::new("survival"));
        assert!(matches!(
            event.result(),
            PlayerChooseInitialServerResult::Redirect(s) if s.as_str() == "survival"
        ));
    }

    #[test]
    fn choose_initial_send_to_limbo_shortcut() {
        let mut event = choose_initial_event();
        event.send_to_limbo(vec!["queue".into()]);
        assert!(matches!(
            event.result(),
            PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers } if limbo_handlers == &["queue".to_string()]
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
