use std::net::SocketAddr;

use super::{GuestEvent, ResultCell};
use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::component::{Component, from_host};
use crate::types::{ServerId, socket_from_wit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandshakeIntent {
    Status,
    Login,
    Transfer,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ConnectionHandshakeResult {
    Allow,
    Deny(Option<Component>),
    DropSilently,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConnectionHandshakeEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub raw_host: String,
    pub port: u16,
    pub protocol: i32,
    pub intent: HandshakeIntent,
    pub legacy: bool,
    pub server: Option<ServerId>,
    result: ResultCell<ConnectionHandshakeResult>,
}

impl ConnectionHandshakeEvent {
    #[must_use]
    pub const fn result(&self) -> &ConnectionHandshakeResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: ConnectionHandshakeResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(ConnectionHandshakeResult::Allow);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(ConnectionHandshakeResult::Deny(Some(reason.into())));
    }

    pub fn deny_silently(&mut self) {
        self.set_result(ConnectionHandshakeResult::Deny(None));
    }

    pub fn drop_silently(&mut self) {
        self.set_result(ConnectionHandshakeResult::DropSilently);
    }
}

impl GuestEvent for ConnectionHandshakeEvent {
    const KIND: EventKind = EventKind::ConnectionHandshake;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ConnectionHandshake(e) = ev else {
            return None;
        };
        Some(Self {
            remote_addr: socket_from_wit(e.remote_addr),
            virtual_host: e.virtual_host,
            raw_host: e.raw_host,
            port: e.port,
            protocol: e.protocol,
            intent: match e.intent {
                we::HandshakeIntent::Status => HandshakeIntent::Status,
                we::HandshakeIntent::Login => HandshakeIntent::Login,
                we::HandshakeIntent::Transfer => HandshakeIntent::Transfer,
            },
            legacy: e.legacy,
            server: e.server.map(ServerId::from),
            result: ResultCell::new(match e.result {
                we::ConnectionHandshakeResult::Allow => ConnectionHandshakeResult::Allow,
                we::ConnectionHandshakeResult::Deny(reason) => {
                    ConnectionHandshakeResult::Deny(reason.map(from_host))
                }
                we::ConnectionHandshakeResult::DropSilently => {
                    ConnectionHandshakeResult::DropSilently
                }
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::ConnectionHandshake(match r {
                    ConnectionHandshakeResult::Allow => we::ConnectionHandshakeResult::Allow,
                    ConnectionHandshakeResult::Deny(reason) => we::ConnectionHandshakeResult::Deny(
                        reason.as_ref().map(Component::to_arena),
                    ),
                    ConnectionHandshakeResult::DropSilently => {
                        we::ConnectionHandshakeResult::DropSilently
                    }
                })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RejectReason {
    IpFilter,
    RateLimit,
    UnknownDomain,
    IpBanned,
    Banned,
    ServerUnavailable,
    Plugin(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectionRejectedEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub reason: RejectReason,
}

impl GuestEvent for ConnectionRejectedEvent {
    const KIND: EventKind = EventKind::ConnectionRejected;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ConnectionRejected(e) = ev else {
            return None;
        };
        Some(Self {
            remote_addr: socket_from_wit(e.remote_addr),
            virtual_host: e.virtual_host,
            reason: match e.reason {
                we::RejectReason::IpFilter => RejectReason::IpFilter,
                we::RejectReason::RateLimit => RejectReason::RateLimit,
                we::RejectReason::UnknownDomain => RejectReason::UnknownDomain,
                we::RejectReason::IpBanned => RejectReason::IpBanned,
                we::RejectReason::Banned => RejectReason::Banned,
                we::RejectReason::ServerUnavailable => RejectReason::ServerUnavailable,
                we::RejectReason::Plugin(plugin) => RejectReason::Plugin(plugin),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::types as wt;

    #[test]
    fn a_handshake_drop_becomes_the_outcome() {
        let mut event = ConnectionHandshakeEvent::from_event(Event::ConnectionHandshake(
            we::ConnectionHandshakeEvent {
                remote_addr: wt::SocketAddress {
                    ip: wt::IpAddress::Ipv4((203, 0, 113, 7)),
                    port: 50_000,
                },
                virtual_host: Some("lobby.test".into()),
                raw_host: "Lobby.Test".into(),
                port: 25565,
                protocol: 767,
                intent: we::HandshakeIntent::Login,
                legacy: false,
                server: Some("lobby".into()),
                result: we::ConnectionHandshakeResult::Allow,
            },
        ))
        .unwrap();
        assert_eq!(event.intent, HandshakeIntent::Login);
        event.drop_silently();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::ConnectionHandshake(we::ConnectionHandshakeResult::DropSilently)
        );
    }
}
