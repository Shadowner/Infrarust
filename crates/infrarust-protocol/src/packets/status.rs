use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::Packet;

// ── SStatusRequest ──────────────────────────────────────────────────

/// Status request packet (Serverbound, 0x00).
///
/// Empty packet sent by the client to request the server's status JSON.
#[derive(Debug, Clone)]
pub struct SStatusRequest;

impl Packet for SStatusRequest {
    const NAME: &'static str = "SStatusRequest";

    fn state() -> ConnectionState {
        ConnectionState::Status
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(_r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        Ok(Self)
    }

    fn encode(
        &self,
        _w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        Ok(())
    }
}

// ── CStatusResponse ────────────────────────────────────────────────

/// Status response packet (Clientbound, 0x00).
///
/// Contains the server's status as a JSON string. The JSON includes version info,
/// player count, description (MOTD), and optional favicon. The proxy treats
/// it as an opaque string — parsing is the responsibility of the layer above.
#[derive(Debug, Clone)]
pub struct CStatusResponse {
    pub json_response: String,
}

impl Packet for CStatusResponse {
    const NAME: &'static str = "CStatusResponse";

    fn state() -> ConnectionState {
        ConnectionState::Status
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let json_response = r.read_string()?;
        Ok(Self { json_response })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_string(&self.json_response)?;
        Ok(())
    }
}

// ── SPingRequest ────────────────────────────────────────────────────

/// Ping request packet (Serverbound, 0x01).
///
/// The client sends a payload (typically a timestamp); the server echoes it back.
#[derive(Debug, Clone)]
pub struct SPingRequest {
    pub payload: i64,
}

impl Packet for SPingRequest {
    const NAME: &'static str = "SPingRequest";

    fn state() -> ConnectionState {
        ConnectionState::Status
    }

    fn direction() -> Direction {
        Direction::Serverbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let payload = r.read_i64_be()?;
        Ok(Self { payload })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_i64_be(self.payload)?;
        Ok(())
    }
}

// ── CPingResponse ───────────────────────────────────────────────────

/// Ping response packet (Clientbound, 0x01).
///
/// Echoes back the client's ping payload.
#[derive(Debug, Clone)]
pub struct CPingResponse {
    pub payload: i64,
}

impl Packet for CPingResponse {
    const NAME: &'static str = "CPingResponse";

    fn state() -> ConnectionState {
        ConnectionState::Status
    }

    fn direction() -> Direction {
        Direction::Clientbound
    }

    fn decode(r: &mut &[u8], _version: ProtocolVersion) -> ProtocolResult<Self> {
        let payload = r.read_i64_be()?;
        Ok(Self { payload })
    }

    fn encode(
        &self,
        mut w: &mut (impl std::io::Write + ?Sized),
        _version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        w.write_i64_be(self.payload)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::registry::build_default_registry;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_status_request_round_trip() {
        let pkt = SStatusRequest;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        assert!(buf.is_empty());
        let decoded = SStatusRequest::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(
            std::mem::size_of_val(&decoded),
            std::mem::size_of::<SStatusRequest>()
        );
    }

    #[test]
    fn test_status_response_round_trip() {
        let json = r#"{"version":{"name":"1.21","protocol":767},"players":{"max":100,"online":5}}"#;
        let pkt = CStatusResponse {
            json_response: json.to_string(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.json_response, json);
    }

    #[test]
    fn test_status_response_large_json() {
        let json = "x".repeat(8192);
        let pkt = CStatusResponse {
            json_response: json.clone(),
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.json_response, json);
    }

    #[test]
    fn test_ping_request_round_trip() {
        let pkt = SPingRequest {
            payload: 1_234_567_890_123_456_789,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.payload, 1_234_567_890_123_456_789);
    }

    #[test]
    fn test_ping_response_round_trip() {
        let pkt = CPingResponse {
            payload: -9_876_543_210,
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.payload, -9_876_543_210);
    }

    #[test]
    fn test_status_packets_in_registry() {
        let registry = build_default_registry();

        // All status packets should be registered for V1_7_2+
        for version in [
            ProtocolVersion::V1_7_2,
            ProtocolVersion::V1_8,
            ProtocolVersion::V1_21,
        ] {
            assert!(
                registry.has_decoder(
                    ConnectionState::Status,
                    Direction::Serverbound,
                    version,
                    0x00,
                ),
                "SStatusRequest should be registered for {version}"
            );

            assert!(
                registry.has_decoder(
                    ConnectionState::Status,
                    Direction::Clientbound,
                    version,
                    0x00,
                ),
                "CStatusResponse should be registered for {version}"
            );

            assert!(
                registry.has_decoder(
                    ConnectionState::Status,
                    Direction::Serverbound,
                    version,
                    0x01,
                ),
                "SPingRequest should be registered for {version}"
            );

            assert!(
                registry.has_decoder(
                    ConnectionState::Status,
                    Direction::Clientbound,
                    version,
                    0x01,
                ),
                "CPingResponse should be registered for {version}"
            );
        }
    }
}
