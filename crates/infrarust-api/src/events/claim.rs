use std::net::SocketAddr;

use bytes::Bytes;
use uuid::Uuid;

use crate::handoff::Handoff;
use crate::types::{ProtocolVersion, ServerId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawConnectionResult {
    #[default]
    Pass,
    NeedMore(u32),
    Handoff(Handoff),
    Reject,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct RawConnectionEvent {
    pub connection_id: u64,
    pub remote_addr: SocketAddr,
    pub local_addr: SocketAddr,
    pub prefix: Bytes,
    pub data: Bytes,
    result: RawConnectionResult,
}

impl RawConnectionEvent {
    pub fn new(
        connection_id: u64,
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
        prefix: impl Into<Bytes>,
        data: impl Into<Bytes>,
    ) -> Self {
        Self {
            connection_id,
            remote_addr,
            local_addr,
            prefix: prefix.into(),
            data: data.into(),
            result: RawConnectionResult::Pass,
        }
    }

    pub fn pass(&mut self) {
        self.result = RawConnectionResult::Pass;
    }

    pub fn need_more(&mut self, bytes: u32) {
        self.result = RawConnectionResult::NeedMore(bytes);
    }

    pub fn handoff(&mut self, handoff: Handoff) {
        self.result = RawConnectionResult::Handoff(handoff);
    }

    pub fn reject(&mut self) {
        self.result = RawConnectionResult::Reject;
    }
}

crate::event::resulted_event!(RawConnectionEvent, RawConnectionResult);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginClaimResult {
    #[default]
    Pass,
    Handoff(Handoff),
}

#[derive(Debug)]
#[non_exhaustive]
pub struct LoginClaimEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub raw_host: String,
    pub port: u16,
    pub protocol_version: ProtocolVersion,
    pub server: Option<ServerId>,
    pub username: String,
    pub uuid: Option<Uuid>,
    pub login_start: Bytes,
    pub layout_matches: bool,
    result: LoginClaimResult,
}

impl LoginClaimEvent {
    pub fn new(
        remote_addr: SocketAddr,
        protocol_version: ProtocolVersion,
        username: impl Into<String>,
        login_start: impl Into<Bytes>,
    ) -> Self {
        Self {
            remote_addr,
            virtual_host: None,
            raw_host: String::new(),
            port: 0,
            protocol_version,
            server: None,
            username: username.into(),
            uuid: None,
            login_start: login_start.into(),
            layout_matches: true,
            result: LoginClaimResult::Pass,
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
    pub const fn with_uuid(mut self, uuid: Option<Uuid>) -> Self {
        self.uuid = uuid;
        self
    }

    #[must_use]
    pub const fn with_layout_matches(mut self, layout_matches: bool) -> Self {
        self.layout_matches = layout_matches;
        self
    }

    pub fn pass(&mut self) {
        self.result = LoginClaimResult::Pass;
    }

    pub fn handoff(&mut self, handoff: Handoff) {
        self.result = LoginClaimResult::Handoff(handoff);
    }
}

crate::event::resulted_event!(LoginClaimEvent, LoginClaimResult);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::event::ResultedEvent;
    use crate::handoff::HandoffTarget;

    fn raw() -> RawConnectionEvent {
        RawConnectionEvent::new(
            42,
            "203.0.113.7:50000".parse().unwrap(),
            "0.0.0.0:25565".parse().unwrap(),
            &b"AMMH"[..],
            &b"AMMH\x00\x01"[..],
        )
    }

    #[test]
    fn a_raw_connection_passes_until_a_listener_decides() {
        let mut event = raw();
        assert_eq!(event.result(), &RawConnectionResult::Pass);
        assert_eq!(event.connection_id, 42);
        assert_eq!(event.prefix, Bytes::from_static(b"AMMH"));
        assert_eq!(event.data.len(), 6);

        event.need_more(64);
        assert_eq!(event.result(), &RawConnectionResult::NeedMore(64));
        event.handoff(Handoff::to(HandoffTarget::server_port("lobby", 30037)));
        assert_eq!(
            event.result(),
            &RawConnectionResult::Handoff(Handoff::to(HandoffTarget::server_port("lobby", 30037)))
        );
        event.reject();
        assert_eq!(event.result(), &RawConnectionResult::Reject);
        event.pass();
        assert_eq!(event.result(), &RawConnectionResult::Pass);
    }

    #[test]
    fn a_login_claim_carries_the_login_start_as_sent() {
        let uuid = Uuid::from_u128(7);
        let mut event = LoginClaimEvent::new(
            "203.0.113.7:50000".parse().unwrap(),
            ProtocolVersion::MINECRAFT_1_21,
            "Steve",
            &b"\x05Steve"[..],
        )
        .with_host("lobby.test\0FML3\0", Some("lobby.test".into()), 25565)
        .with_server(Some(ServerId::new("lobby")))
        .with_uuid(Some(uuid))
        .with_layout_matches(false);
        assert_eq!(event.result(), &LoginClaimResult::Pass);
        assert_eq!(event.raw_host, "lobby.test\0FML3\0");
        assert_eq!(event.virtual_host.as_deref(), Some("lobby.test"));
        assert_eq!(event.port, 25565);
        assert_eq!(event.server, Some(ServerId::new("lobby")));
        assert_eq!(event.uuid, Some(uuid));
        assert_eq!(event.login_start, Bytes::from_static(b"\x05Steve"));
        assert!(!event.layout_matches);

        event.handoff(Handoff::route().without_replay());
        assert_eq!(
            event.result(),
            &LoginClaimResult::Handoff(Handoff::route().without_replay())
        );
        event.pass();
        assert_eq!(event.result(), &LoginClaimResult::Pass);
    }
}
