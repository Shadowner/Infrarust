use infrarust_api::event::ResultedEvent;
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

const fn intent(intent: HandshakeIntent) -> we::HandshakeIntent {
    match intent {
        HandshakeIntent::Status => we::HandshakeIntent::Status,
        HandshakeIntent::Transfer => we::HandshakeIntent::Transfer,
        _ => we::HandshakeIntent::Login,
    }
}

impl WasmEvent for ConnectionHandshakeEvent {
    const KIND: EventKind = EventKind::ConnectionHandshake;

    fn to_wit(&self) -> we::Event {
        we::Event::ConnectionHandshake(we::ConnectionHandshakeEvent {
            remote_addr: convert::socket_to_wit(self.remote_addr),
            virtual_host: self.virtual_host.clone(),
            raw_host: self.raw_host.clone(),
            port: self.port,
            protocol: self.protocol_version.raw(),
            intent: intent(self.intent),
            legacy: self.legacy,
            server: self.server.as_ref().map(|s| s.as_str().to_owned()),
            result: match self.result() {
                ConnectionHandshakeResult::Deny { reason } => {
                    we::ConnectionHandshakeResult::Deny(reason.as_ref().map(component::to_wit))
                }
                ConnectionHandshakeResult::DropSilently => {
                    we::ConnectionHandshakeResult::DropSilently
                }
                _ => we::ConnectionHandshakeResult::Allow,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::ConnectionHandshake(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::ConnectionHandshakeResult::Allow => ConnectionHandshakeResult::Allow,
            we::ConnectionHandshakeResult::Deny(reason) => ConnectionHandshakeResult::Deny {
                reason: reason.as_ref().map(|reason| texts.convert(reason)),
            },
            we::ConnectionHandshakeResult::DropSilently => ConnectionHandshakeResult::DropSilently,
        });
        texts.applied()
    }
}

fn reject_reason(reason: &RejectReason) -> we::RejectReason {
    match reason {
        RejectReason::IpFilter => we::RejectReason::IpFilter,
        RejectReason::RateLimit => we::RejectReason::RateLimit,
        RejectReason::UnknownDomain => we::RejectReason::UnknownDomain,
        RejectReason::IpBanned => we::RejectReason::IpBanned,
        RejectReason::Banned => we::RejectReason::Banned,
        RejectReason::ServerUnavailable => we::RejectReason::ServerUnavailable,
        RejectReason::Plugin { plugin_id } => we::RejectReason::Plugin(plugin_id.clone()),
        _ => we::RejectReason::Plugin(None),
    }
}

impl WasmEvent for ConnectionRejectedEvent {
    const KIND: EventKind = EventKind::ConnectionRejected;

    fn to_wit(&self) -> we::Event {
        we::Event::ConnectionRejected(we::ConnectionRejectedEvent {
            remote_addr: convert::socket_to_wit(self.remote_addr),
            virtual_host: self.virtual_host.clone(),
            reason: reject_reason(&self.reason),
        })
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::types::{Component, ProtocolVersion, ServerId};

    use super::*;

    fn handshake() -> ConnectionHandshakeEvent {
        ConnectionHandshakeEvent::new(
            "203.0.113.7:50000".parse().unwrap(),
            HandshakeIntent::Transfer,
            ProtocolVersion::new(767),
        )
        .with_host("Lobby.Test\0FML3\0", Some("lobby.test".into()), 25565)
        .with_server(Some(ServerId::new("lobby")))
        .with_legacy(false)
    }

    #[test]
    fn the_guest_sees_the_raw_and_the_resolved_host() {
        let we::Event::ConnectionHandshake(record) = handshake().to_wit() else {
            panic!("a handshake is sent as connection-handshake");
        };
        assert_eq!(record.raw_host, "Lobby.Test\0FML3\0");
        assert_eq!(record.virtual_host.as_deref(), Some("lobby.test"));
        assert_eq!(record.port, 25565);
        assert_eq!(record.intent, we::HandshakeIntent::Transfer);
        assert_eq!(record.server.as_deref(), Some("lobby"));
        assert_eq!(record.result, we::ConnectionHandshakeResult::Allow);
    }

    #[test]
    fn each_handshake_result_sets_the_native_one() {
        let mut event = handshake();
        event.apply(we::EventOutcome::ConnectionHandshake(
            we::ConnectionHandshakeResult::DropSilently,
        ));
        assert_eq!(event.result(), &ConnectionHandshakeResult::DropSilently);
        let reason = component::to_wit(&Component::text("bots go home"));
        event.apply(we::EventOutcome::ConnectionHandshake(
            we::ConnectionHandshakeResult::Deny(Some(reason)),
        ));
        assert_eq!(
            event.result(),
            &ConnectionHandshakeResult::Deny {
                reason: Some(Component::text("bots go home"))
            }
        );
        event.apply(we::EventOutcome::Unchanged);
        assert!(matches!(
            event.result(),
            ConnectionHandshakeResult::Deny { .. }
        ));
    }

    #[test]
    fn a_rejection_names_the_plugin_that_refused() {
        let event = ConnectionRejectedEvent::new(
            "203.0.113.7:50000".parse().unwrap(),
            None,
            RejectReason::Plugin {
                plugin_id: Some("gate".into()),
            },
        );
        let we::Event::ConnectionRejected(record) = event.to_wit() else {
            panic!("a rejection is sent as connection-rejected");
        };
        assert_eq!(record.reason, we::RejectReason::Plugin(Some("gate".into())));
        assert_eq!(record.virtual_host, None);
    }
}
