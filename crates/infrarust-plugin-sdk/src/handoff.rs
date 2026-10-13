use crate::bindings::events as we;
use crate::bindings::types as wt;
use crate::types::ServerId;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandoffTarget {
    Route,
    Server(ServerId),
    ServerPort { server: ServerId, port: u16 },
    Address { host: String, port: u16 },
}

impl HandoffTarget {
    pub fn server(server: impl Into<ServerId>) -> Self {
        Self::Server(server.into())
    }

    pub fn server_port(server: impl Into<ServerId>, port: u16) -> Self {
        Self::ServerPort {
            server: server.into(),
            port,
        }
    }

    pub fn address(host: impl Into<String>, port: u16) -> Self {
        Self::Address {
            host: host.into(),
            port,
        }
    }

    fn to_wit(&self) -> we::HandoffTarget {
        match self {
            Self::Route => we::HandoffTarget::Route,
            Self::Server(server) => we::HandoffTarget::Server(server.as_str().to_owned()),
            Self::ServerPort { server, port } => {
                we::HandoffTarget::ServerPort((server.as_str().to_owned(), *port))
            }
            Self::Address { host, port } => we::HandoffTarget::Address(wt::ServerAddress {
                host: host.clone(),
                port: *port,
            }),
        }
    }

    fn from_wit(target: we::HandoffTarget) -> Self {
        match target {
            we::HandoffTarget::Route => Self::Route,
            we::HandoffTarget::Server(server) => Self::Server(ServerId::from(server)),
            we::HandoffTarget::ServerPort((server, port)) => Self::ServerPort {
                server: ServerId::from(server),
                port,
            },
            we::HandoffTarget::Address(address) => Self::Address {
                host: address.host,
                port: address.port,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Handoff {
    pub target: HandoffTarget,
    pub replay: bool,
    pub prefix: Vec<u8>,
    pub reply: Vec<u8>,
    pub proxy_protocol: Option<bool>,
}

impl Handoff {
    #[must_use]
    pub const fn to(target: HandoffTarget) -> Self {
        Self {
            target,
            replay: true,
            prefix: Vec::new(),
            reply: Vec::new(),
            proxy_protocol: None,
        }
    }

    #[must_use]
    pub const fn route() -> Self {
        Self::to(HandoffTarget::Route)
    }

    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<Vec<u8>>) -> Self {
        self.prefix = prefix.into();
        self
    }

    #[must_use]
    pub fn with_reply(mut self, reply: impl Into<Vec<u8>>) -> Self {
        self.reply = reply.into();
        self
    }

    #[must_use]
    pub const fn without_replay(mut self) -> Self {
        self.replay = false;
        self
    }

    #[must_use]
    pub const fn with_replay(mut self, replay: bool) -> Self {
        self.replay = replay;
        self
    }

    #[must_use]
    pub const fn with_proxy_protocol(mut self, enabled: bool) -> Self {
        self.proxy_protocol = Some(enabled);
        self
    }

    pub(crate) fn to_wit(&self) -> we::Handoff {
        we::Handoff {
            target: self.target.to_wit(),
            replay: self.replay,
            prefix: self.prefix.clone(),
            reply: self.reply.clone(),
            proxy_protocol: self.proxy_protocol,
        }
    }

    pub(crate) fn from_wit(handoff: we::Handoff) -> Self {
        Self {
            target: HandoffTarget::from_wit(handoff.target),
            replay: handoff.replay,
            prefix: handoff.prefix,
            reply: handoff.reply,
            proxy_protocol: handoff.proxy_protocol,
        }
    }
}

impl From<HandoffTarget> for Handoff {
    fn from(target: HandoffTarget) -> Self {
        Self::to(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handoff_replays_by_default_and_crosses_the_contract_unchanged() {
        let route = Handoff::route();
        assert!(route.replay);
        assert_eq!(route.proxy_protocol, None);

        for handoff in [
            route,
            Handoff::to(HandoffTarget::server("modded")).without_replay(),
            Handoff::to(HandoffTarget::server_port("modded", 30037)).with_prefix(b"AM".to_vec()),
            Handoff::to(HandoffTarget::address("files.example", 443))
                .with_reply(b"ok".to_vec())
                .with_proxy_protocol(true)
                .with_replay(false),
        ] {
            assert_eq!(Handoff::from_wit(handoff.to_wit()), handoff);
        }
        assert_eq!(
            Handoff::from(HandoffTarget::Route),
            Handoff::to(HandoffTarget::Route)
        );
    }
}
