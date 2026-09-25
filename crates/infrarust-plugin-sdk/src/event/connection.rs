use super::{GuestEvent, ResultCell};
use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::component::{Component, from_host};
use crate::types::{PlayerRef, ServerId};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerChooseInitialServerResult {
    Allowed,
    Redirect(ServerId),
    SendToLimbo(Vec<String>),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlayerChooseInitialServerEvent {
    pub player: PlayerRef,
    pub initial_server: ServerId,
    result: ResultCell<PlayerChooseInitialServerResult>,
}

impl PlayerChooseInitialServerEvent {
    #[must_use]
    pub const fn result(&self) -> &PlayerChooseInitialServerResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: PlayerChooseInitialServerResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(PlayerChooseInitialServerResult::Allowed);
    }

    pub fn redirect_to(&mut self, server: impl Into<ServerId>) {
        self.set_result(PlayerChooseInitialServerResult::Redirect(server.into()));
    }

    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.set_result(PlayerChooseInitialServerResult::SendToLimbo(handlers));
    }
}

impl GuestEvent for PlayerChooseInitialServerEvent {
    const KIND: EventKind = EventKind::PlayerChooseInitialServer;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::PlayerChooseInitialServer(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            initial_server: ServerId::from(e.initial_server),
            result: ResultCell::new(match e.result {
                we::PlayerChooseInitialServerResult::Allowed => {
                    PlayerChooseInitialServerResult::Allowed
                }
                we::PlayerChooseInitialServerResult::Redirect(server) => {
                    PlayerChooseInitialServerResult::Redirect(ServerId::from(server))
                }
                we::PlayerChooseInitialServerResult::SendToLimbo(handlers) => {
                    PlayerChooseInitialServerResult::SendToLimbo(handlers)
                }
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::PlayerChooseInitialServer(match r {
                    PlayerChooseInitialServerResult::Allowed => {
                        we::PlayerChooseInitialServerResult::Allowed
                    }
                    PlayerChooseInitialServerResult::Redirect(server) => {
                        we::PlayerChooseInitialServerResult::Redirect(server.into_string())
                    }
                    PlayerChooseInitialServerResult::SendToLimbo(handlers) => {
                        we::PlayerChooseInitialServerResult::SendToLimbo(handlers)
                    }
                })
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConnectCause {
    Initial,
    Switch,
    LimboExit,
    KickRedirect,
    PluginMessage,
}

impl ConnectCause {
    const fn from_wit(cause: we::ConnectCause) -> Self {
        match cause {
            we::ConnectCause::Initial => Self::Initial,
            we::ConnectCause::Switch => Self::Switch,
            we::ConnectCause::LimboExit => Self::LimboExit,
            we::ConnectCause::KickRedirect => Self::KickRedirect,
            we::ConnectCause::PluginMessage => Self::PluginMessage,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ServerPreConnectResult {
    Allowed,
    ConnectTo(ServerId),
    SendToLimbo(Vec<String>),
    Denied(Component),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerPreConnectEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
    pub cause: ConnectCause,
    result: ResultCell<ServerPreConnectResult>,
}

impl ServerPreConnectEvent {
    #[must_use]
    pub const fn result(&self) -> &ServerPreConnectResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: ServerPreConnectResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(ServerPreConnectResult::Allowed);
    }

    pub fn redirect_to(&mut self, server: impl Into<ServerId>) {
        self.set_result(ServerPreConnectResult::ConnectTo(server.into()));
    }

    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.set_result(ServerPreConnectResult::SendToLimbo(handlers));
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(ServerPreConnectResult::Denied(reason.into()));
    }
}

impl GuestEvent for ServerPreConnectEvent {
    const KIND: EventKind = EventKind::ServerPreConnect;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ServerPreConnect(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            server: ServerId::from(e.server),
            previous_server: e.previous_server.map(ServerId::from),
            cause: ConnectCause::from_wit(e.cause),
            result: ResultCell::new(match e.result {
                we::ServerPreConnectResult::Allowed => ServerPreConnectResult::Allowed,
                we::ServerPreConnectResult::ConnectTo(server) => {
                    ServerPreConnectResult::ConnectTo(ServerId::from(server))
                }
                we::ServerPreConnectResult::SendToLimbo(handlers) => {
                    ServerPreConnectResult::SendToLimbo(handlers)
                }
                we::ServerPreConnectResult::Denied(reason) => {
                    ServerPreConnectResult::Denied(from_host(reason))
                }
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::ServerPreConnect(match r {
                    ServerPreConnectResult::Allowed => we::ServerPreConnectResult::Allowed,
                    ServerPreConnectResult::ConnectTo(server) => {
                        we::ServerPreConnectResult::ConnectTo(server.into_string())
                    }
                    ServerPreConnectResult::SendToLimbo(handlers) => {
                        we::ServerPreConnectResult::SendToLimbo(handlers)
                    }
                    ServerPreConnectResult::Denied(reason) => {
                        we::ServerPreConnectResult::Denied(reason.to_arena())
                    }
                })
            })
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerConnectedEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
}

impl GuestEvent for ServerConnectedEvent {
    const KIND: EventKind = EventKind::ServerConnected;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ServerConnected(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            server: ServerId::from(e.server),
            previous_server: e.previous_server.map(ServerId::from),
        })
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ServerPostConnectEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub previous_server: Option<ServerId>,
}

impl ServerPostConnectEvent {
    #[must_use]
    pub fn switched_from(&self) -> Option<&ServerId> {
        self.previous_server
            .as_ref()
            .filter(|previous| **previous != self.server)
    }
}

impl GuestEvent for ServerPostConnectEvent {
    const KIND: EventKind = EventKind::ServerPostConnect;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ServerPostConnect(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            server: ServerId::from(e.server),
            previous_server: e.previous_server.map(ServerId::from),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KickCause {
    Unreachable(String),
    LoginRefused,
    ConfigDisconnect,
    PlayDisconnect,
    ConnectionLost,
}

impl KickCause {
    fn from_wit(cause: we::KickCause) -> Self {
        match cause {
            we::KickCause::Unreachable(error) => Self::Unreachable(error),
            we::KickCause::LoginRefused => Self::LoginRefused,
            we::KickCause::ConfigDisconnect => Self::ConfigDisconnect,
            we::KickCause::PlayDisconnect => Self::PlayDisconnect,
            we::KickCause::ConnectionLost => Self::ConnectionLost,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum KickedFromServerResult {
    DisconnectPlayer(Option<Component>),
    RedirectTo(ServerId),
    SendToLimbo(Vec<String>),
    Notify(Component),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct KickedFromServerEvent {
    pub player: PlayerRef,
    pub server: ServerId,
    pub reason: Option<Component>,
    pub cause: KickCause,
    pub during_connect: bool,
    pub previous_server: Option<ServerId>,
    result: ResultCell<KickedFromServerResult>,
}

impl KickedFromServerEvent {
    #[must_use]
    pub const fn result(&self) -> &KickedFromServerResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: KickedFromServerResult) {
        self.result.set(result);
    }

    pub fn disconnect(&mut self, reason: impl Into<Component>) {
        self.set_result(KickedFromServerResult::DisconnectPlayer(Some(
            reason.into(),
        )));
    }

    pub fn redirect_to(&mut self, server: impl Into<ServerId>) {
        self.set_result(KickedFromServerResult::RedirectTo(server.into()));
    }

    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.set_result(KickedFromServerResult::SendToLimbo(handlers));
    }

    pub fn notify(&mut self, message: impl Into<Component>) {
        self.set_result(KickedFromServerResult::Notify(message.into()));
    }
}

impl GuestEvent for KickedFromServerEvent {
    const KIND: EventKind = EventKind::KickedFromServer;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::KickedFromServer(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            server: ServerId::from(e.server),
            reason: e.reason.map(from_host),
            cause: KickCause::from_wit(e.cause),
            during_connect: e.during_connect,
            previous_server: e.previous_server.map(ServerId::from),
            result: ResultCell::new(match e.result {
                we::KickedFromServerResult::DisconnectPlayer(reason) => {
                    KickedFromServerResult::DisconnectPlayer(reason.map(from_host))
                }
                we::KickedFromServerResult::RedirectTo(server) => {
                    KickedFromServerResult::RedirectTo(ServerId::from(server))
                }
                we::KickedFromServerResult::SendToLimbo(handlers) => {
                    KickedFromServerResult::SendToLimbo(handlers)
                }
                we::KickedFromServerResult::Notify(message) => {
                    KickedFromServerResult::Notify(from_host(message))
                }
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::KickedFromServer(match r {
                    KickedFromServerResult::DisconnectPlayer(reason) => {
                        we::KickedFromServerResult::DisconnectPlayer(
                            reason.as_ref().map(Component::to_arena),
                        )
                    }
                    KickedFromServerResult::RedirectTo(server) => {
                        we::KickedFromServerResult::RedirectTo(server.into_string())
                    }
                    KickedFromServerResult::SendToLimbo(handlers) => {
                        we::KickedFromServerResult::SendToLimbo(handlers)
                    }
                    KickedFromServerResult::Notify(message) => {
                        we::KickedFromServerResult::Notify(message.to_arena())
                    }
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::types as wt;

    fn steve() -> wt::PlayerRef {
        wt::PlayerRef {
            id: 1,
            uuid: wt::Uuid { hi: 0, lo: 1 },
            username: "Steve".into(),
        }
    }

    #[test]
    fn a_redirect_is_sent_and_an_untouched_pre_connect_is_unchanged() {
        let event = || {
            ServerPreConnectEvent::from_event(Event::ServerPreConnect(we::ServerPreConnectEvent {
                player: steve(),
                server: "lobby".into(),
                previous_server: None,
                cause: we::ConnectCause::Initial,
                result: we::ServerPreConnectResult::Allowed,
            }))
            .unwrap()
        };
        assert_eq!(event().into_outcome(), EventOutcome::Unchanged);
        let mut redirected = event();
        assert_eq!(redirected.cause, ConnectCause::Initial);
        redirected.redirect_to("backend-2");
        assert_eq!(
            redirected.into_outcome(),
            EventOutcome::ServerPreConnect(we::ServerPreConnectResult::ConnectTo(
                "backend-2".into()
            ))
        );
    }

    #[test]
    fn a_kick_decodes_its_cause_reason_and_current_result() {
        let event =
            KickedFromServerEvent::from_event(Event::KickedFromServer(we::KickedFromServerEvent {
                player: steve(),
                server: "survival".into(),
                reason: None,
                cause: we::KickCause::Unreachable("refused".into()),
                during_connect: true,
                previous_server: Some("lobby".into()),
                result: we::KickedFromServerResult::DisconnectPlayer(None),
            }))
            .unwrap();
        assert_eq!(event.cause, KickCause::Unreachable("refused".into()));
        assert_eq!(
            event.result(),
            &KickedFromServerResult::DisconnectPlayer(None)
        );
        assert_eq!(event.previous_server, Some(ServerId::from("lobby")));
    }

    #[test]
    fn a_post_connect_knows_whether_it_is_a_switch() {
        let post = |previous: Option<&str>| {
            ServerPostConnectEvent::from_event(Event::ServerPostConnect(
                we::ServerPostConnectEvent {
                    player: steve(),
                    server: "survival".into(),
                    previous_server: previous.map(str::to_owned),
                },
            ))
            .unwrap()
        };
        assert_eq!(post(None).switched_from(), None);
        assert_eq!(post(Some("survival")).switched_from(), None);
        assert_eq!(
            post(Some("lobby")).switched_from(),
            Some(&ServerId::from("lobby"))
        );
    }
}
