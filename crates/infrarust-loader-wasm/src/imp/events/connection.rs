use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::types::ServerId;

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

fn server(id: &ServerId) -> String {
    id.as_str().to_owned()
}

fn previous(id: Option<&ServerId>) -> Option<String> {
    id.map(server)
}

impl WasmEvent for PlayerChooseInitialServerEvent {
    const KIND: EventKind = EventKind::PlayerChooseInitialServer;

    fn to_wit(&self) -> we::Event {
        we::Event::PlayerChooseInitialServer(we::PlayerChooseInitialServerEvent {
            player: convert::player_ref(&*self.player),
            initial_server: server(&self.initial_server),
            result: match self.result() {
                PlayerChooseInitialServerResult::Redirect(target) => {
                    we::PlayerChooseInitialServerResult::Redirect(server(target))
                }
                PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers } => {
                    we::PlayerChooseInitialServerResult::SendToLimbo(limbo_handlers.clone())
                }
                _ => we::PlayerChooseInitialServerResult::Allowed,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::PlayerChooseInitialServer(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::PlayerChooseInitialServerResult::Allowed => {
                PlayerChooseInitialServerResult::Allowed
            }
            we::PlayerChooseInitialServerResult::Redirect(target) => {
                PlayerChooseInitialServerResult::Redirect(ServerId::from(target))
            }
            we::PlayerChooseInitialServerResult::SendToLimbo(limbo_handlers) => {
                PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers }
            }
        });
        Applied::Set
    }
}

fn connect_cause(cause: ConnectCause) -> we::ConnectCause {
    match cause {
        ConnectCause::Switch => we::ConnectCause::Switch,
        ConnectCause::LimboExit => we::ConnectCause::LimboExit,
        ConnectCause::KickRedirect => we::ConnectCause::KickRedirect,
        ConnectCause::PluginMessage => we::ConnectCause::PluginMessage,
        _ => we::ConnectCause::Initial,
    }
}

impl WasmEvent for ServerPreConnectEvent {
    const KIND: EventKind = EventKind::ServerPreConnect;

    fn to_wit(&self) -> we::Event {
        we::Event::ServerPreConnect(we::ServerPreConnectEvent {
            player: convert::player_ref(&*self.player),
            server: server(&self.server),
            previous_server: previous(self.previous_server.as_ref()),
            cause: connect_cause(self.cause),
            result: match self.result() {
                ServerPreConnectResult::ConnectTo(target) => {
                    we::ServerPreConnectResult::ConnectTo(server(target))
                }
                ServerPreConnectResult::SendToLimbo { limbo_handlers } => {
                    we::ServerPreConnectResult::SendToLimbo(limbo_handlers.clone())
                }
                ServerPreConnectResult::Denied { reason } => {
                    we::ServerPreConnectResult::Denied(component::to_wit(reason))
                }
                _ => we::ServerPreConnectResult::Allowed,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::ServerPreConnect(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::ServerPreConnectResult::Allowed => ServerPreConnectResult::Allowed,
            we::ServerPreConnectResult::ConnectTo(target) => {
                ServerPreConnectResult::ConnectTo(ServerId::from(target))
            }
            we::ServerPreConnectResult::SendToLimbo(limbo_handlers) => {
                ServerPreConnectResult::SendToLimbo { limbo_handlers }
            }
            we::ServerPreConnectResult::Denied(reason) => ServerPreConnectResult::Denied {
                reason: texts.convert(&reason),
            },
        });
        texts.applied()
    }
}

impl WasmEvent for ServerConnectedEvent {
    const KIND: EventKind = EventKind::ServerConnected;

    fn to_wit(&self) -> we::Event {
        we::Event::ServerConnected(we::ServerConnectedEvent {
            player: convert::player_ref(&*self.player),
            server: server(&self.server),
            previous_server: previous(self.previous_server.as_ref()),
        })
    }
}

impl WasmEvent for ServerPostConnectEvent {
    const KIND: EventKind = EventKind::ServerPostConnect;

    fn to_wit(&self) -> we::Event {
        we::Event::ServerPostConnect(we::ServerPostConnectEvent {
            player: convert::player_ref(&*self.player),
            server: server(&self.server),
            previous_server: previous(self.previous_server.as_ref()),
        })
    }
}

fn kick_cause(cause: &KickCause) -> we::KickCause {
    match cause {
        KickCause::Unreachable { error } => we::KickCause::Unreachable(error.clone()),
        KickCause::LoginRefused => we::KickCause::LoginRefused,
        KickCause::ConfigDisconnect => we::KickCause::ConfigDisconnect,
        KickCause::ConnectionLost => we::KickCause::ConnectionLost,
        _ => we::KickCause::PlayDisconnect,
    }
}

impl WasmEvent for KickedFromServerEvent {
    const KIND: EventKind = EventKind::KickedFromServer;

    fn to_wit(&self) -> we::Event {
        we::Event::KickedFromServer(we::KickedFromServerEvent {
            player: convert::player_ref(&*self.player),
            server: server(&self.server),
            reason: self.reason.as_ref().map(component::to_wit),
            cause: kick_cause(&self.cause),
            during_connect: self.during_connect,
            previous_server: previous(self.previous_server.as_ref()),
            result: match self.result() {
                KickedFromServerResult::RedirectTo(target) => {
                    we::KickedFromServerResult::RedirectTo(server(target))
                }
                KickedFromServerResult::SendToLimbo { limbo_handlers } => {
                    we::KickedFromServerResult::SendToLimbo(limbo_handlers.clone())
                }
                KickedFromServerResult::Notify { message } => {
                    we::KickedFromServerResult::Notify(component::to_wit(message))
                }
                KickedFromServerResult::DisconnectPlayer { reason } => {
                    we::KickedFromServerResult::DisconnectPlayer(
                        reason.as_ref().map(component::to_wit),
                    )
                }
                _ => we::KickedFromServerResult::DisconnectPlayer(None),
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::KickedFromServer(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::KickedFromServerResult::DisconnectPlayer(reason) => {
                KickedFromServerResult::DisconnectPlayer {
                    reason: reason.as_ref().map(|reason| texts.convert(reason)),
                }
            }
            we::KickedFromServerResult::RedirectTo(target) => {
                KickedFromServerResult::RedirectTo(ServerId::from(target))
            }
            we::KickedFromServerResult::SendToLimbo(limbo_handlers) => {
                KickedFromServerResult::SendToLimbo { limbo_handlers }
            }
            we::KickedFromServerResult::Notify(message) => KickedFromServerResult::Notify {
                message: texts.convert(&message),
            },
        });
        texts.applied()
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::types::Component;

    use super::super::steve;
    use super::*;

    fn post_connect(target: &str, from: Option<&str>) -> we::ServerPostConnectEvent {
        let event =
            ServerPostConnectEvent::new(steve(), ServerId::new(target), from.map(ServerId::new));
        let we::Event::ServerPostConnect(record) = event.to_wit() else {
            panic!("a post-connect is sent as server-post-connect");
        };
        record
    }

    #[test]
    fn every_post_connect_reaches_the_guest_with_its_previous_server() {
        let first = post_connect("lobby", None);
        assert_eq!(first.player.id, 1);
        assert_eq!(first.server, "lobby");
        assert_eq!(first.previous_server, None);

        let switch = post_connect("survival", Some("lobby"));
        assert_eq!(switch.server, "survival");
        assert_eq!(switch.previous_server.as_deref(), Some("lobby"));
    }

    fn kicked() -> KickedFromServerEvent {
        KickedFromServerEvent::new(
            steve(),
            ServerId::new("survival"),
            None,
            KickCause::Unreachable {
                error: "refused".into(),
            },
            true,
            Some(ServerId::new("lobby")),
            KickedFromServerResult::default(),
        )
    }

    #[test]
    fn a_kick_carries_its_cause_and_the_default_result() {
        let we::Event::KickedFromServer(record) = kicked().to_wit() else {
            panic!("a kick is sent as kicked-from-server");
        };
        assert_eq!(record.reason, None);
        assert_eq!(record.cause, we::KickCause::Unreachable("refused".into()));
        assert!(record.during_connect);
        assert_eq!(record.previous_server.as_deref(), Some("lobby"));
        assert_eq!(
            record.result,
            we::KickedFromServerResult::DisconnectPlayer(None)
        );
    }

    #[test]
    fn a_notify_outcome_sets_the_native_result() {
        let mut event = kicked();
        let message = component::to_wit(&Component::text("moved"));
        assert_eq!(
            event.apply(we::EventOutcome::KickedFromServer(
                we::KickedFromServerResult::Notify(message)
            )),
            Applied::Set
        );
        assert_eq!(
            event.result(),
            &KickedFromServerResult::Notify {
                message: Component::text("moved")
            }
        );
    }

    #[test]
    fn a_pre_connect_sees_its_cause_and_ignores_a_foreign_outcome() {
        let mut event = ServerPreConnectEvent::new(
            steve(),
            ServerId::new("survival"),
            Some(ServerId::new("lobby")),
            ConnectCause::Switch,
        );
        let we::Event::ServerPreConnect(record) = event.to_wit() else {
            panic!("a pre-connect is sent as server-pre-connect");
        };
        assert_eq!(record.cause, we::ConnectCause::Switch);
        assert_eq!(
            event.apply(we::EventOutcome::PreLogin(we::PreLoginResult::ForceOffline)),
            Applied::Mismatched
        );
        assert_eq!(event.result(), &ServerPreConnectResult::Allowed);
    }
}
