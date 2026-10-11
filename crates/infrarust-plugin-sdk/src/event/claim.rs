use std::net::SocketAddr;

use uuid::Uuid;

use super::ResultCell;
use crate::bindings::events as we;
use crate::handoff::Handoff;
use crate::types::{ServerId, socket_from_wit, uuid_from_wit};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawConnectionResult {
    Pass,
    NeedMore(u32),
    Handoff(Handoff),
    Reject,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RawConnectionEvent {
    pub connection_id: u64,
    pub remote_addr: SocketAddr,
    pub local_addr: SocketAddr,
    pub prefix: Vec<u8>,
    pub data: Vec<u8>,
    result: ResultCell<RawConnectionResult>,
}

impl RawConnectionEvent {
    #[must_use]
    pub const fn result(&self) -> &RawConnectionResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: RawConnectionResult) {
        self.result.set(result);
    }

    pub fn pass(&mut self) {
        self.set_result(RawConnectionResult::Pass);
    }

    pub fn need_more(&mut self, bytes: u32) {
        self.set_result(RawConnectionResult::NeedMore(bytes));
    }

    pub fn handoff(&mut self, handoff: Handoff) {
        self.set_result(RawConnectionResult::Handoff(handoff));
    }

    pub fn reject(&mut self) {
        self.set_result(RawConnectionResult::Reject);
    }
}

guest_event!(
    RawConnectionEvent,
    RawConnection,
    |e| Self {
        connection_id: e.connection_id,
        remote_addr: socket_from_wit(e.remote_addr),
        local_addr: socket_from_wit(e.local_addr),
        prefix: e.prefix,
        data: e.data,
        result: ResultCell::new(match e.result {
            we::RawConnectionResult::Pass => RawConnectionResult::Pass,
            we::RawConnectionResult::NeedMore(bytes) => RawConnectionResult::NeedMore(bytes),
            we::RawConnectionResult::Handoff(handoff) => {
                RawConnectionResult::Handoff(Handoff::from_wit(handoff))
            }
            we::RawConnectionResult::Reject => RawConnectionResult::Reject,
        }),
    },
    result,
    |r| match r {
        RawConnectionResult::Pass => we::RawConnectionResult::Pass,
        RawConnectionResult::NeedMore(bytes) => we::RawConnectionResult::NeedMore(bytes),
        RawConnectionResult::Handoff(handoff) => we::RawConnectionResult::Handoff(handoff.to_wit()),
        RawConnectionResult::Reject => we::RawConnectionResult::Reject,
    }
);

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoginClaimResult {
    Pass,
    Handoff(Handoff),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoginClaimEvent {
    pub remote_addr: SocketAddr,
    pub virtual_host: Option<String>,
    pub raw_host: String,
    pub port: u16,
    pub protocol: i32,
    pub server: Option<ServerId>,
    pub username: String,
    pub uuid: Option<Uuid>,
    pub login_start: Vec<u8>,
    pub layout_matches: bool,
    result: ResultCell<LoginClaimResult>,
}

impl LoginClaimEvent {
    #[must_use]
    pub const fn result(&self) -> &LoginClaimResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: LoginClaimResult) {
        self.result.set(result);
    }

    pub fn pass(&mut self) {
        self.set_result(LoginClaimResult::Pass);
    }

    pub fn handoff(&mut self, handoff: Handoff) {
        self.set_result(LoginClaimResult::Handoff(handoff));
    }
}

guest_event!(
    LoginClaimEvent,
    LoginClaim,
    |e| Self {
        remote_addr: socket_from_wit(e.remote_addr),
        virtual_host: e.virtual_host,
        raw_host: e.raw_host,
        port: e.port,
        protocol: e.protocol,
        server: e.server.map(ServerId::from),
        username: e.username,
        uuid: e.uuid.map(uuid_from_wit),
        login_start: e.login_start,
        layout_matches: e.layout_matches,
        result: ResultCell::new(match e.result {
            we::LoginClaimResult::Pass => LoginClaimResult::Pass,
            we::LoginClaimResult::Handoff(handoff) => {
                LoginClaimResult::Handoff(Handoff::from_wit(handoff))
            }
        }),
    },
    result,
    |r| match r {
        LoginClaimResult::Pass => we::LoginClaimResult::Pass,
        LoginClaimResult::Handoff(handoff) => we::LoginClaimResult::Handoff(handoff.to_wit()),
    }
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::bindings::events::{Event, EventOutcome};
    use crate::bindings::types as wt;
    use crate::event::GuestEvent;
    use crate::handoff::HandoffTarget;

    fn socket(port: u16) -> wt::SocketAddress {
        wt::SocketAddress {
            ip: wt::IpAddress::Ipv4((203, 0, 113, 7)),
            port,
        }
    }

    fn raw(result: we::RawConnectionResult) -> RawConnectionEvent {
        RawConnectionEvent::from_event(Event::RawConnection(we::RawConnectionEvent {
            connection_id: 3,
            remote_addr: socket(50_000),
            local_addr: socket(25_565),
            prefix: b"AMMH".to_vec(),
            data: b"AMMH\x00\x01".to_vec(),
            result,
        }))
        .unwrap()
    }

    #[test]
    fn an_untouched_raw_connection_answers_unchanged() {
        let event = raw(we::RawConnectionResult::NeedMore(8));
        assert_eq!(event.connection_id, 3);
        assert_eq!(event.prefix, b"AMMH");
        assert_eq!(event.data, b"AMMH\x00\x01");
        assert_eq!(event.local_addr.port(), 25_565);
        assert_eq!(event.result(), &RawConnectionResult::NeedMore(8));
        assert_eq!(event.into_outcome(), EventOutcome::Unchanged);
    }

    #[test]
    fn each_raw_connection_helper_becomes_the_outcome() {
        let mut event = raw(we::RawConnectionResult::Pass);
        event.need_more(64);
        assert_eq!(
            event.into_outcome(),
            EventOutcome::RawConnection(we::RawConnectionResult::NeedMore(64))
        );

        let mut event = raw(we::RawConnectionResult::Pass);
        event.reject();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::RawConnection(we::RawConnectionResult::Reject)
        );

        let mut event = raw(we::RawConnectionResult::Pass);
        event.handoff(
            Handoff::to(HandoffTarget::server_port("modded", 30037))
                .with_prefix(b"PRE".to_vec())
                .without_replay(),
        );
        assert_eq!(
            event.into_outcome(),
            EventOutcome::RawConnection(we::RawConnectionResult::Handoff(we::Handoff {
                target: we::HandoffTarget::ServerPort(("modded".into(), 30037)),
                replay: false,
                prefix: b"PRE".to_vec(),
                reply: Vec::new(),
                proxy_protocol: None,
            }))
        );

        let mut event = raw(we::RawConnectionResult::Reject);
        event.pass();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::RawConnection(we::RawConnectionResult::Pass)
        );
    }

    #[test]
    fn a_login_claim_hands_the_login_off() {
        let mut event = LoginClaimEvent::from_event(Event::LoginClaim(we::LoginClaimEvent {
            remote_addr: socket(50_000),
            virtual_host: Some("modded.test".into()),
            raw_host: "modded.test\0FML3\0".into(),
            port: 25565,
            protocol: 767,
            server: Some("modded".into()),
            username: "Steve".into(),
            uuid: Some(wt::Uuid { hi: 0, lo: 7 }),
            login_start: b"\x05Steve".to_vec(),
            layout_matches: false,
            result: we::LoginClaimResult::Pass,
        }))
        .unwrap();
        assert_eq!(event.username, "Steve");
        assert_eq!(event.uuid, Some(Uuid::from_u64_pair(0, 7)));
        assert_eq!(event.server, Some(ServerId::new("modded")));
        assert!(!event.layout_matches);
        assert_eq!(event.result(), &LoginClaimResult::Pass);

        event.handoff(Handoff::route().with_proxy_protocol(false));
        assert_eq!(
            event.into_outcome(),
            EventOutcome::LoginClaim(we::LoginClaimResult::Handoff(we::Handoff {
                target: we::HandoffTarget::Route,
                replay: true,
                prefix: Vec::new(),
                reply: Vec::new(),
                proxy_protocol: Some(false),
            }))
        );
    }
}
