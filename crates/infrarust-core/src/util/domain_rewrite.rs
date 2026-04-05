//! Centralized domain rewrite logic for handshake packets.
//!
//! Replaces the copy-pasted domain rewrite code from V1.
//! Used by `BackendBridge::send_initial_packets()` and handlers.

use infrarust_config::{DomainRewrite, ServerConfig};
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::version::ConnectionState;
use infrarust_protocol::{Packet, VarInt};

use crate::error::CoreError;
use crate::pipeline::types::{ConnectionIntent, HandshakeData};

/// Rewrites the handshake packet domain according to the server config.
///
/// Returns the raw bytes of the (possibly rewritten) handshake packet,
/// framed and ready to write to the backend stream.
///
/// - `DomainRewrite::None` → returns the original handshake bytes as-is
/// - `DomainRewrite::Explicit(domain)` → replaces `server_address` with `domain`
/// - `DomainRewrite::FromBackend` → replaces with the first backend address
///
/// # Errors
/// Returns `CoreError` if handshake packet encoding fails.
pub fn rewrite_handshake(
    handshake_data: &HandshakeData,
    server_config: &ServerConfig,
) -> Result<Vec<u8>, CoreError> {
    match &server_config.domain_rewrite {
        DomainRewrite::None => Ok(first_raw_packet(handshake_data)),
        DomainRewrite::Explicit(domain) => encode_handshake_with_domain(handshake_data, domain),
        DomainRewrite::FromBackend => server_config.addresses.first().map_or_else(
            || Ok(first_raw_packet(handshake_data)),
            |addr| encode_handshake_with_domain(handshake_data, &addr.host),
        ),
        _ => {
            // Future non-exhaustive variants: pass through
            Ok(first_raw_packet(handshake_data))
        }
    }
}

fn first_raw_packet(handshake_data: &HandshakeData) -> Vec<u8> {
    handshake_data
        .raw_packets
        .first()
        .map(|b| b.to_vec())
        .unwrap_or_default()
}

pub(crate) fn encode_handshake_with_domain(
    handshake_data: &HandshakeData,
    new_domain: &str,
) -> Result<Vec<u8>, CoreError> {
    let next_state = match handshake_data.intent {
        ConnectionIntent::Status => ConnectionState::Status,
        ConnectionIntent::Login | ConnectionIntent::Transfer => ConnectionState::Login,
    };

    let modified = SHandshake {
        protocol_version: VarInt(handshake_data.protocol_version.0),
        server_address: new_domain.to_string(),
        server_port: handshake_data.port,
        next_state,
    };

    let mut payload = Vec::new();
    modified.encode(&mut payload, handshake_data.protocol_version)?;

    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, &payload)?;
    Ok(encoder.take().to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::default_trait_access)]
    use super::*;
    use bytes::BytesMut;
    use infrarust_config::ServerAddress;
    use infrarust_protocol::version::ProtocolVersion;

    fn make_handshake_data() -> HandshakeData {
        // Build a raw handshake packet for "play.example.com"
        let handshake = SHandshake {
            protocol_version: VarInt(ProtocolVersion::V1_21.0),
            server_address: "play.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };
        let mut payload = Vec::new();
        handshake
            .encode(&mut payload, ProtocolVersion::V1_21)
            .unwrap();
        let mut encoder = PacketEncoder::new();
        encoder.append_raw(0x00, &payload).unwrap();
        let raw = encoder.take();

        HandshakeData {
            domain: "play.example.com".to_string(),
            port: 25565,
            protocol_version: ProtocolVersion::V1_21,
            intent: ConnectionIntent::Login,
            raw_packets: vec![BytesMut::from(&raw[..])],
        }
    }

    fn make_server_config(rewrite: DomainRewrite) -> ServerConfig {
        ServerConfig {
            id: Some("test".to_string()),
            name: None,
            network: None,
            domains: vec!["play.example.com".to_string()],
            addresses: vec!["backend.local:25565".parse::<ServerAddress>().unwrap()],
            proxy_mode: Default::default(),
            forwarding_mode: None,
            send_proxy_protocol: false,
            domain_rewrite: rewrite,
            motd: Default::default(),
            server_manager: None,
            timeouts: None,
            max_players: 0,
            ip_filter: None,
            disconnect_message: None,
            limbo_handlers: vec![],
        }
    }

    #[test]
    fn test_rewrite_none() {
        let hd = make_handshake_data();
        let config = make_server_config(DomainRewrite::None);
        let result = rewrite_handshake(&hd, &config).unwrap();
        // Should return original bytes unchanged
        assert_eq!(result, hd.raw_packets[0].to_vec());
    }

    #[test]
    fn test_rewrite_explicit() {
        let hd = make_handshake_data();
        let config = make_server_config(DomainRewrite::Explicit("new.domain.com".to_string()));
        let result = rewrite_handshake(&hd, &config).unwrap();
        // Should differ from original (different domain encoded)
        assert_ne!(result, hd.raw_packets[0].to_vec());
        // Decode the result to verify domain was replaced
        let decoded = decode_handshake_from_framed(&result);
        assert_eq!(decoded.server_address, "new.domain.com");
        assert_eq!(decoded.server_port, 25565);
    }

    #[test]
    fn test_rewrite_from_backend() {
        let hd = make_handshake_data();
        let config = make_server_config(DomainRewrite::FromBackend);
        let result = rewrite_handshake(&hd, &config).unwrap();
        let decoded = decode_handshake_from_framed(&result);
        assert_eq!(decoded.server_address, "backend.local");
    }

    /// Decode a framed handshake packet back into `SHandshake`.
    fn decode_handshake_from_framed(data: &[u8]) -> SHandshake {
        use infrarust_protocol::io::PacketDecoder;
        let mut decoder = PacketDecoder::new();
        decoder.queue_bytes(data);
        let frame = decoder.try_next_frame().unwrap().unwrap();
        assert_eq!(frame.id, 0x00);
        let mut payload = &frame.payload[..];
        SHandshake::decode(&mut payload, ProtocolVersion::V1_21).unwrap()
    }
}
