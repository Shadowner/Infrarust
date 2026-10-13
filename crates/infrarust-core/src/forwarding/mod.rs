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
    /// Returns whether this handler uses Velocity modern forwarding.
    pub const fn is_velocity(&self) -> bool {
        matches!(self, Self::Velocity(_))
    }

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
    use infrarust_protocol::VarInt;
    use infrarust_protocol::version::ConnectionState;

    use crate::pipeline::types::ConnectionIntent;

    let host = crate::util::domain_rewrite::backend_host(server_config)
        .unwrap_or_else(|| handshake_data.domain.clone());

    let next_state = match handshake_data.intent {
        ConnectionIntent::Status => ConnectionState::Status,
        ConnectionIntent::Login | ConnectionIntent::Transfer => ConnectionState::Login,
    };

    infrarust_protocol::packets::handshake::SHandshake {
        protocol_version: VarInt(handshake_data.protocol_version.0),
        server_address: format!("{host}{}", handshake_data.host_suffix()),
        server_port: handshake_data.port,
        next_state,
        transfer: handshake_data.intent == ConnectionIntent::Transfer,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use bytes::BytesMut;
    use infrarust_config::{DomainRewrite, ServerConfig};
    use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
    use infrarust_protocol::packets::handshake::SHandshake;
    use infrarust_protocol::version::ConnectionState;
    use infrarust_protocol::{Packet, VarInt};
    use serde_json::Value;

    use super::*;
    use crate::pipeline::types::{ConnectionIntent, HandshakeData};
    use crate::util::domain_rewrite::rewrite_handshake;

    const MARKERS: [&str; 4] = ["\0FML\0", "\0FML2\0", "\0FML3\0", "\0FORGE"];
    const PLAYER_UUID: &str = "069a79f444e94726a5befca90e38aaf5";

    fn handshake_data(raw_host: &str) -> HandshakeData {
        let version = ProtocolVersion::V1_20_2;
        let handshake = SHandshake {
            protocol_version: VarInt(version.0),
            server_address: raw_host.to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
            transfer: false,
        };
        let mut payload = Vec::new();
        handshake.encode(&mut payload, version).unwrap();
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, &payload).unwrap();
        HandshakeData {
            domain: crate::util::normalize_handshake(raw_host).to_lowercase(),
            raw_host: raw_host.to_string(),
            port: 25565,
            protocol_version: version,
            intent: ConnectionIntent::Login,
            raw_packets: vec![BytesMut::from(&encoder.take()[..])],
        }
    }

    fn server_config(rewrite: DomainRewrite) -> ServerConfig {
        let mut config: ServerConfig =
            toml::from_str("domains = [\"lobby.test\"]\naddresses = [\"backend.local:25565\"]\n")
                .unwrap();
        config.domain_rewrite = rewrite;
        config
    }

    fn forwarding_data() -> ForwardingData {
        ForwardingData {
            real_ip: "192.168.1.42".parse::<IpAddr>().unwrap(),
            uuid: uuid::Uuid::parse_str(PLAYER_UUID).unwrap(),
            username: "Notch".to_string(),
            properties: vec![],
            protocol_version: ProtocolVersion::V1_20_2,
            chat_session: None,
        }
    }

    fn sent_host(
        handshake: &HandshakeData,
        rewrite: DomainRewrite,
        mode: &ForwardingMode,
    ) -> String {
        let config = server_config(rewrite);
        let handler = build_forwarding_handler(mode);
        if handler.modifies_handshake() {
            let mut modified = build_handshake_for_backend(handshake, &config);
            handler.apply_handshake(&mut modified, &forwarding_data());
            return modified.server_address;
        }
        let framed = rewrite_handshake(handshake, &config).unwrap();
        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(&framed);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        SHandshake::decode(&mut frame.payload.as_ref(), handshake.protocol_version)
            .unwrap()
            .server_address
    }

    fn rewrites() -> [(DomainRewrite, &'static str); 3] {
        [
            (DomainRewrite::None, "lobby.test"),
            (
                DomainRewrite::Explicit("backend.example".to_string()),
                "backend.example",
            ),
            (DomainRewrite::FromBackend, "backend.local"),
        ]
    }

    fn handshake_forwarding_modes() -> [ForwardingMode; 2] {
        [
            ForwardingMode::BungeeCord,
            ForwardingMode::BungeeGuard {
                token: "guard".to_string(),
            },
        ]
    }

    fn legacy_properties(sent: &str, host: &str) -> Vec<Value> {
        let parts: Vec<&str> = sent.split('\0').collect();
        assert_eq!(parts.len(), 4, "{parts:?}");
        assert_eq!(parts[0], host);
        assert_eq!(parts[1], "192.168.1.42");
        assert_eq!(parts[2], PLAYER_UUID);
        serde_json::from_str(parts[3]).unwrap()
    }

    fn count(properties: &[Value], name: &str) -> usize {
        properties.iter().filter(|p| p["name"] == name).count()
    }

    #[test]
    fn markers_survive_without_handshake_forwarding() {
        let modes = [
            ForwardingMode::None,
            ForwardingMode::Velocity {
                secret: b"secret".to_vec(),
            },
        ];
        for marker in MARKERS {
            let handshake = handshake_data(&format!("Lobby.Test.{marker}"));
            for mode in &modes {
                for (rewrite, host) in rewrites() {
                    let expected = match rewrite {
                        DomainRewrite::None => handshake.raw_host.clone(),
                        _ => format!("{host}{marker}"),
                    };
                    assert_eq!(sent_host(&handshake, rewrite, mode), expected, "{mode:?}");
                }
            }
        }
    }

    #[test]
    fn markers_become_properties_under_legacy_forwarding() {
        for marker in MARKERS {
            let handshake = handshake_data(&format!("Lobby.Test.{marker}"));
            for mode in &handshake_forwarding_modes() {
                for (rewrite, host) in rewrites() {
                    let properties = legacy_properties(&sent_host(&handshake, rewrite, mode), host);
                    let extra_data = marker.replace('\0', "\u{1}");
                    assert!(
                        properties
                            .iter()
                            .any(|p| p["name"] == "forgeClient" && p["value"] == "true")
                    );
                    assert!(
                        properties
                            .iter()
                            .any(|p| p["name"] == "extraData" && p["value"] == extra_data.as_str())
                    );
                    let guarded = matches!(mode, ForwardingMode::BungeeGuard { .. });
                    assert_eq!(
                        count(&properties, "bungeeguard-token"),
                        usize::from(guarded)
                    );
                }
            }
        }
    }

    #[test]
    fn a_vanilla_host_gets_no_forge_properties() {
        let handshake = handshake_data("lobby.test");
        for mode in &handshake_forwarding_modes() {
            for (rewrite, host) in rewrites() {
                let properties = legacy_properties(&sent_host(&handshake, rewrite, mode), host);
                assert_eq!(count(&properties, "forgeClient"), 0);
                assert_eq!(count(&properties, "extraData"), 0);
            }
        }
        for (rewrite, host) in rewrites() {
            assert_eq!(sent_host(&handshake, rewrite, &ForwardingMode::None), host);
        }
    }

    #[test]
    fn a_forged_host_cannot_inject_legacy_forwarding_fields() {
        let handshake = handshake_data(
            "lobby.test\0FML2\0\x00198.51.100.7\x000123456789abcdef0123456789abcdef",
        );
        for mode in &handshake_forwarding_modes() {
            for (rewrite, host) in rewrites() {
                let sent = sent_host(&handshake, rewrite, mode);
                assert_eq!(sent.matches('\0').count(), 3, "{sent:?}");
                let properties = legacy_properties(&sent, host);
                assert_eq!(count(&properties, "extraData"), 1);
            }
        }
    }
}
