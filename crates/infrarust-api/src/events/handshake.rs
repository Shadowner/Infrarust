use std::net::SocketAddr;

use crate::event::{Event, ResultedEvent};
use crate::types::{Component, ProtocolVersion, ServerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HandshakeIntent {
    Status,
    Login,
    Transfer,
}

impl HandshakeIntent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Login => "login",
            Self::Transfer => "transfer",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum ConnectionHandshakeResult {
    #[default]
    Allow,
    Deny {
        reason: Option<Component>,
    },
    DropSilently,
}

#[non_exhaustive]
pub struct ConnectionHandshakeEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub raw_host: String,
    pub port: u16,
    pub protocol_version: ProtocolVersion,
    pub intent: HandshakeIntent,
    pub legacy: bool,
    pub server: Option<ServerId>,
    result: ConnectionHandshakeResult,
}

impl ConnectionHandshakeEvent {
    pub const fn new(
        remote_addr: SocketAddr,
        intent: HandshakeIntent,
        protocol_version: ProtocolVersion,
    ) -> Self {
        Self {
            remote_addr,
            virtual_host: None,
            raw_host: String::new(),
            port: 0,
            protocol_version,
            intent,
            legacy: false,
            server: None,
            result: ConnectionHandshakeResult::Allow,
        }
    }

    #[must_use]
    pub fn with_host(
        mut self,
        raw_host: impl Into<String>,
        virtual_host: Option<String>,
        port: u16,
    ) -> Self {
        self.raw_host = raw_host.into();
        self.virtual_host = virtual_host;
        self.port = port;
        self
    }

    #[must_use]
    pub fn with_server(mut self, server: Option<ServerId>) -> Self {
        self.server = server;
        self
    }

    #[must_use]
    pub const fn with_legacy(mut self, legacy: bool) -> Self {
        self.legacy = legacy;
        self
    }

    pub fn allow(&mut self) {
        self.result = ConnectionHandshakeResult::Allow;
    }

    pub fn deny(&mut self, reason: Component) {
        self.result = ConnectionHandshakeResult::Deny {
            reason: Some(reason),
        };
    }

    pub fn drop_silently(&mut self) {
        self.result = ConnectionHandshakeResult::DropSilently;
    }
}

impl Event for ConnectionHandshakeEvent {}

impl ResultedEvent for ConnectionHandshakeEvent {
    type Result = ConnectionHandshakeResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
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
    Plugin { plugin_id: Option<String> },
}

impl RejectReason {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::IpFilter => "ip_filter",
            Self::RateLimit => "rate_limit",
            Self::UnknownDomain => "unknown_domain",
            Self::IpBanned => "ip_banned",
            Self::Banned => "banned",
            Self::ServerUnavailable => "server_unavailable",
            Self::Plugin { .. } => "plugin",
        }
    }
}

#[non_exhaustive]
pub struct ConnectionRejectedEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub reason: RejectReason,
}

impl ConnectionRejectedEvent {
    pub const fn new(
        remote_addr: SocketAddr,
        virtual_host: Option<String>,
        reason: RejectReason,
    ) -> Self {
        Self {
            remote_addr,
            virtual_host,
            reason,
        }
    }
}

impl Event for ConnectionRejectedEvent {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn event() -> ConnectionHandshakeEvent {
        ConnectionHandshakeEvent::new(
            "203.0.113.7:50000".parse().unwrap(),
            HandshakeIntent::Login,
            ProtocolVersion::MINECRAFT_1_21,
        )
        .with_host("Lobby.Test\0FML3\0", Some("lobby.test".into()), 25565)
        .with_server(Some(ServerId::new("lobby")))
    }

    #[test]
    fn a_handshake_is_allowed_until_a_listener_decides() {
        let mut event = event();
        assert_eq!(event.result(), &ConnectionHandshakeResult::Allow);
        assert_eq!(event.raw_host, "Lobby.Test\0FML3\0");
        assert_eq!(event.virtual_host.as_deref(), Some("lobby.test"));
        assert!(!event.legacy);

        event.deny(Component::text("bots go home"));
        assert_eq!(
            event.result(),
            &ConnectionHandshakeResult::Deny {
                reason: Some(Component::text("bots go home"))
            }
        );
        event.drop_silently();
        assert_eq!(event.result(), &ConnectionHandshakeResult::DropSilently);
        event.allow();
        assert_eq!(event.result(), &ConnectionHandshakeResult::Allow);
    }

    #[test]
    fn reject_reasons_have_stable_names() {
        assert_eq!(RejectReason::IpFilter.as_str(), "ip_filter");
        assert_eq!(RejectReason::UnknownDomain.as_str(), "unknown_domain");
        assert_eq!(RejectReason::Plugin { plugin_id: None }.as_str(), "plugin");
        assert_eq!(HandshakeIntent::Transfer.as_str(), "transfer");
    }
}
