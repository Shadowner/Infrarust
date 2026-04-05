//! Player identity forwarding for Minecraft backend servers.

pub mod bungeeguard;
pub mod error;
pub mod legacy;
pub mod secret;
pub mod velocity;

pub use error::ForwardingError;

use std::net::IpAddr;

use infrarust_api::types::ProfileProperty;
use infrarust_protocol::version::ProtocolVersion;

use self::bungeeguard::BungeeGuardForwardingHandler;
use self::legacy::LegacyForwardingHandler;
use self::velocity::VelocityForwardingHandler;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ForwardingMode {
    #[default]
    None,
    BungeeCord,
    BungeeGuard {
        token: String,
    },
    Velocity {
        secret: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct ForwardingData {
    pub real_ip: IpAddr,
    pub uuid: uuid::Uuid,
    pub username: String,
    pub properties: Vec<ProfileProperty>,
    pub protocol_version: ProtocolVersion,
    pub chat_session: Option<ChatSessionData>,
}

#[derive(Debug, Clone)]
pub struct ChatSessionData {
    pub expiry: i64,
    pub public_key: Vec<u8>,
    pub key_signature: Vec<u8>,
    pub holder_uuid: Option<uuid::Uuid>,
}

pub enum ForwardingHandler {
    None,
    Legacy(LegacyForwardingHandler),
    BungeeGuard(BungeeGuardForwardingHandler),
    Velocity(VelocityForwardingHandler),
}

impl ForwardingHandler {
    pub const fn modifies_handshake(&self) -> bool {
        matches!(self, Self::Legacy(_) | Self::BungeeGuard(_))
    }

    pub fn apply_handshake(
        &self,
        handshake: &mut infrarust_protocol::packets::handshake::SHandshake,
        data: &ForwardingData,
    ) {
        match self {
            Self::None | Self::Velocity(_) => {}
            Self::Legacy(h) => h.apply_handshake(handshake, data),
            Self::BungeeGuard(h) => h.apply_handshake(handshake, data),
        }
    }
}

pub fn build_handshake_for_backend(
    handshake_data: &crate::pipeline::types::HandshakeData,
    server_config: &infrarust_config::ServerConfig,
) -> infrarust_protocol::packets::handshake::SHandshake {
    use infrarust_config::DomainRewrite;
    use infrarust_protocol::VarInt;
    use infrarust_protocol::version::ConnectionState;

    use crate::pipeline::types::ConnectionIntent;

    let domain = match &server_config.domain_rewrite {
        DomainRewrite::None => handshake_data.domain.clone(),
        DomainRewrite::Explicit(d) => d.clone(),
        DomainRewrite::FromBackend => server_config
            .addresses
            .first()
            .map(|a| a.host.clone())
            .unwrap_or_else(|| handshake_data.domain.clone()),
        _ => handshake_data.domain.clone(),
    };

    let next_state = match handshake_data.intent {
        ConnectionIntent::Status => ConnectionState::Status,
        ConnectionIntent::Login | ConnectionIntent::Transfer => ConnectionState::Login,
    };

    infrarust_protocol::packets::handshake::SHandshake {
        protocol_version: VarInt(handshake_data.protocol_version.0),
        server_address: domain,
        server_port: handshake_data.port,
        next_state,
    }
}

pub fn build_forwarding_handler(mode: &ForwardingMode) -> ForwardingHandler {
    match mode {
        ForwardingMode::None => ForwardingHandler::None,
        ForwardingMode::BungeeCord => ForwardingHandler::Legacy(LegacyForwardingHandler),
        ForwardingMode::BungeeGuard { token } => {
            ForwardingHandler::BungeeGuard(BungeeGuardForwardingHandler::new(token.clone()))
        }
        ForwardingMode::Velocity { .. } => ForwardingHandler::Velocity(VelocityForwardingHandler),
    }
}
