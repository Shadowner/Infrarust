use bytes::Bytes;
use infrarust_plugin_common::ErrorKind;

use crate::types::ServerId;

pub const MIN_CLAIM_PREFIX_LEN: usize = 2;

pub const MAX_CLAIM_PREFIX_LEN: usize = 16;

pub const MAX_CLAIM_PREFIXES: usize = 16;

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

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Server(_) => "server",
            Self::ServerPort { .. } => "server-port",
            Self::Address { .. } => "address",
        }
    }
}

impl std::fmt::Display for HandoffTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Route => f.write_str("route"),
            Self::Server(server) => write!(f, "server {server}"),
            Self::ServerPort { server, port } => write!(f, "server {server} port {port}"),
            Self::Address { host, port } => write!(f, "{host}:{port}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Handoff {
    pub target: HandoffTarget,
    pub replay: bool,
    pub prefix: Bytes,
    pub reply: Bytes,
    pub proxy_protocol: Option<bool>,
}

impl Handoff {
    pub const fn to(target: HandoffTarget) -> Self {
        Self {
            target,
            replay: true,
            prefix: Bytes::new(),
            reply: Bytes::new(),
            proxy_protocol: None,
        }
    }

    pub const fn route() -> Self {
        Self::to(HandoffTarget::Route)
    }

    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<Bytes>) -> Self {
        self.prefix = prefix.into();
        self
    }

    #[must_use]
    pub fn with_reply(mut self, reply: impl Into<Bytes>) -> Self {
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
}

impl From<HandoffTarget> for Handoff {
    fn from(target: HandoffTarget) -> Self {
        Self::to(target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClaimError {
    #[error("invalid connection claim prefix: {0}")]
    InvalidPrefix(&'static str),
    #[error("a plugin holds at most {0} connection claim prefixes")]
    Limit(usize),
    #[error("this plugin holds no such connection claim prefix")]
    NotFound,
}

impl ClaimError {
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidPrefix(_) => ErrorKind::InvalidArgument,
            Self::Limit(_) => ErrorKind::LimitExceeded,
            Self::NotFound => ErrorKind::NotFound,
        }
    }
}

pub mod private {
    pub trait Sealed {}
}

pub trait ConnectionClaimRegistry: Send + Sync + private::Sealed {
    fn register_prefix(&self, prefix: &[u8]) -> Result<(), ClaimError>;

    fn unregister_prefix(&self, prefix: &[u8]) -> Result<(), ClaimError>;
}

pub fn validate_claim_prefix(prefix: &[u8]) -> Result<(), ClaimError> {
    if prefix.len() < MIN_CLAIM_PREFIX_LEN {
        return Err(ClaimError::InvalidPrefix("shorter than 2 bytes"));
    }
    if prefix.len() > MAX_CLAIM_PREFIX_LEN {
        return Err(ClaimError::InvalidPrefix("longer than 16 bytes"));
    }
    let reason = match prefix {
        [0xFE, ..] => Some("0xFE starts a legacy server list ping"),
        [0x02, ..] => Some("0x02 starts a legacy login"),
        [0x01..=0x7F, 0x00, ..] => Some("it can start a Minecraft handshake"),
        [0x80..=0xFF, 0x01..=0x03] | [0x80..=0xFF, 0x01..=0x03, 0x00, ..] => {
            Some("it can start a Minecraft handshake")
        }
        _ if starts_like(prefix, PROXY_V1_SIGNATURE) => {
            Some("it can start a PROXY protocol v1 header")
        }
        _ if starts_like(prefix, PROXY_V2_SIGNATURE) => {
            Some("it can start a PROXY protocol v2 header")
        }
        _ => None,
    };
    reason.map_or(Ok(()), |reason| Err(ClaimError::InvalidPrefix(reason)))
}

const PROXY_V1_SIGNATURE: &[u8] = b"PROXY ";

const PROXY_V2_SIGNATURE: &[u8] = b"\r\n\r\n\x00\r\nQUIT\n";

fn starts_like(prefix: &[u8], signature: &[u8]) -> bool {
    let shared = prefix.len().min(signature.len());
    prefix[..shared] == signature[..shared]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_handoff_replays_by_default_and_builds_up() {
        let handoff = Handoff::route();
        assert_eq!(handoff.target, HandoffTarget::Route);
        assert!(handoff.replay);
        assert!(handoff.prefix.is_empty());
        assert!(handoff.reply.is_empty());
        assert_eq!(handoff.proxy_protocol, None);

        let handoff = Handoff::to(HandoffTarget::server_port("lobby", 30037))
            .with_prefix(&b"AMMH"[..])
            .with_reply(vec![1, 2, 3])
            .without_replay()
            .with_proxy_protocol(true);
        assert_eq!(
            handoff.target,
            HandoffTarget::ServerPort {
                server: ServerId::new("lobby"),
                port: 30037
            }
        );
        assert!(!handoff.replay);
        assert_eq!(handoff.prefix, Bytes::from_static(b"AMMH"));
        assert_eq!(handoff.reply, Bytes::from_static(&[1, 2, 3]));
        assert_eq!(handoff.proxy_protocol, Some(true));
        assert!(handoff.with_replay(true).replay);
    }

    #[test]
    fn targets_name_themselves() {
        assert_eq!(HandoffTarget::Route.as_str(), "route");
        assert_eq!(HandoffTarget::server("hub").to_string(), "server hub");
        assert_eq!(
            HandoffTarget::server_port("hub", 8080).to_string(),
            "server hub port 8080"
        );
        assert_eq!(
            HandoffTarget::address("files.example", 443).to_string(),
            "files.example:443"
        );
        assert_eq!(
            Handoff::from(HandoffTarget::server("hub")),
            Handoff::to(HandoffTarget::Server(ServerId::new("hub")))
        );
    }

    #[test]
    fn prefixes_that_cannot_open_a_minecraft_connection_are_accepted() {
        for prefix in [
            &b"AMMH"[..],
            b"AMOK",
            &[0x00, 0x00],
            &[0x00, 0x01, 0x02],
            &[0x80, 0x04, 0x00],
            &[0x80, 0x01, 0x01],
            &[0x7F, 0x01],
            b"GET ",
            &[0xAB; 16],
        ] {
            assert_eq!(validate_claim_prefix(prefix), Ok(()), "{prefix:?}");
        }
    }

    #[test]
    fn prefixes_that_could_open_a_minecraft_connection_are_refused() {
        for prefix in [
            &[0xFE, 0x01][..],
            &[0x02, 0x41],
            &[0x10, 0x00],
            &[0x7F, 0x00, 0x12],
            &[0x80, 0x01, 0x00],
            &[0x80, 0x02],
            &[0xFF, 0x03, 0x00, 0x00],
            b"PROXY TCP4",
            b"PR",
            b"\r\n\r\n",
            b"\r\n\r\n\x00\r\nQUIT\n\x21",
        ] {
            assert!(
                matches!(
                    validate_claim_prefix(prefix),
                    Err(ClaimError::InvalidPrefix(_))
                ),
                "{prefix:?}"
            );
        }
    }

    #[test]
    fn prefixes_must_be_two_to_sixteen_bytes() {
        assert!(matches!(
            validate_claim_prefix(b"A"),
            Err(ClaimError::InvalidPrefix(_))
        ));
        assert!(matches!(
            validate_claim_prefix(b""),
            Err(ClaimError::InvalidPrefix(_))
        ));
        assert!(matches!(
            validate_claim_prefix(&[0x41; 17]),
            Err(ClaimError::InvalidPrefix(_))
        ));
        assert_eq!(validate_claim_prefix(&[0x41; 2]), Ok(()));
        assert_eq!(
            ClaimError::Limit(16).to_string(),
            "a plugin holds at most 16 connection claim prefixes"
        );
        assert_eq!(
            ClaimError::InvalidPrefix("x").kind(),
            ErrorKind::InvalidArgument
        );
        assert_eq!(ClaimError::Limit(16).kind(), ErrorKind::LimitExceeded);
        assert_eq!(ClaimError::NotFound.kind(), ErrorKind::NotFound);
    }
}
