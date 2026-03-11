use crate::codec::{McBufReadExt, McBufWriteExt, VarInt};
use crate::error::{ProtocolError, ProtocolResult};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::Packet;

/// Handshake packet (Serverbound, 0x00).
///
/// Always the first packet sent by the client. Indicates:
/// - The client's protocol version
/// - The target server address (used by the proxy for domain-based routing)
/// - The server port
/// - The intent: Status (ping) or Login (connection)
///
/// Format stable since Minecraft 1.7, no versioning necessary.
///
/// The server address may contain Forge/FML markers:
/// `"play.example.com\0FML\0"` or `"play.example.com\0FML2\0"`.
/// The proxy must preserve them during forwarding.
#[derive(Debug, Clone)]
pub struct SHandshake {
    /// Client's protocol version (e.g., 767 for MC 1.21).
    pub protocol_version: VarInt,
    /// Server address as entered by the player.
    /// May contain FML markers. Max 255 chars.
    pub server_address: String,
    /// Server port (rarely used by proxies, but present in the protocol).
    pub server_port: u16,
    /// Client's intent: Status (1) or Login (2) or Transfer (3, since 1.20.5).
    pub next_state: ConnectionState,
}

impl Packet for SHandshake {
    const NAME: &'static str = "SHandshake";

    fn state() -> ConnectionState {
        ConnectionState::Handshake
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let protocol_version = r.read_var_int()?;
        let server_address = r.read_string_bounded(255)?;
        let server_port = r.read_u16_be()?;
        let next_state_id = r.read_var_int()?;

        let next_state = ConnectionState::from_handshake_id(next_state_id.0).ok_or_else(|| {
            ProtocolError::invalid(format!("invalid handshake next_state: {}", next_state_id.0))
        })?;

        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_var_int(&self.protocol_version)?;
        w.write_string(&self.server_address)?;
        w.write_u16_be(self.server_port)?;

        let state_id = self.next_state.handshake_id().ok_or_else(|| {
            ProtocolError::invalid(format!(
                "cannot encode handshake next_state: {}",
                self.next_state
            ))
        })?;
        w.write_var_int(&VarInt(state_id))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::ErasedPacket;

    fn encode_handshake(hs: &SHandshake) -> Vec<u8> {
        let mut buf = Vec::new();
        hs.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        buf
    }

    #[test]
    fn test_handshake_round_trip() {
        let original = SHandshake {
            protocol_version: VarInt(767),
            server_address: "play.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let encoded = encode_handshake(&original);
        let decoded = SHandshake::decode(&mut encoded.as_slice(), ProtocolVersion::V1_21).unwrap();

        assert_eq!(decoded.protocol_version, VarInt(767));
        assert_eq!(decoded.server_address, "play.example.com");
        assert_eq!(decoded.server_port, 25565);
        assert_eq!(decoded.next_state, ConnectionState::Login);
    }

    #[test]
    fn test_handshake_status_intent() {
        let hs = SHandshake {
            protocol_version: VarInt(767),
            server_address: "mc.server.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Status,
        };

        let encoded = encode_handshake(&hs);
        let decoded = SHandshake::decode(&mut encoded.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.next_state, ConnectionState::Status);
    }

    #[test]
    fn test_handshake_login_intent() {
        let hs = SHandshake {
            protocol_version: VarInt(767),
            server_address: "mc.server.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let encoded = encode_handshake(&hs);
        let decoded = SHandshake::decode(&mut encoded.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.next_state, ConnectionState::Login);
    }

    #[test]
    fn test_handshake_transfer_intent() {
        // Transfer (intent 3) maps to Login in from_handshake_id
        // We need to manually encode intent 3 to test decoding
        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(767)).unwrap();
        buf.write_string("mc.server.com").unwrap();
        buf.write_u16_be(25565).unwrap();
        buf.write_var_int(&VarInt(3)).unwrap(); // Transfer intent

        let decoded = SHandshake::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.next_state, ConnectionState::Login);
    }

    #[test]
    fn test_handshake_invalid_intent() {
        let mut buf = Vec::new();
        buf.write_var_int(&VarInt(767)).unwrap();
        buf.write_string("mc.server.com").unwrap();
        buf.write_u16_be(25565).unwrap();
        buf.write_var_int(&VarInt(99)).unwrap(); // Invalid intent

        let result = SHandshake::decode(&mut buf.as_slice(), ProtocolVersion::V1_21);
        assert!(result.is_err());
    }

    #[test]
    fn test_handshake_fml_marker() {
        let address = "play.example.com\0FML\0";
        let hs = SHandshake {
            protocol_version: VarInt(767),
            server_address: address.to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let encoded = encode_handshake(&hs);
        let decoded = SHandshake::decode(&mut encoded.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.server_address, address);
    }

    #[test]
    fn test_handshake_protocol_version() {
        let hs = SHandshake {
            protocol_version: VarInt(767),
            server_address: "mc.server.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let encoded = encode_handshake(&hs);
        let decoded = SHandshake::decode(&mut encoded.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(decoded.protocol_version, VarInt(767));
    }

    #[test]
    fn test_handshake_erased_packet_downcast() {
        let hs = SHandshake {
            protocol_version: VarInt(767),
            server_address: "play.example.com".to_string(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        let erased: Box<dyn ErasedPacket> = Box::new(hs);
        assert_eq!(erased.packet_name(), "SHandshake");

        let downcasted = erased.as_any().downcast_ref::<SHandshake>().unwrap();
        assert_eq!(downcasted.protocol_version, VarInt(767));
        assert_eq!(downcasted.server_address, "play.example.com");
        assert_eq!(downcasted.server_port, 25565);
        assert_eq!(downcasted.next_state, ConnectionState::Login);
    }
}
