use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::version::{ConnectionState, Direction, ProtocolVersion};

use super::{Packet, PacketMapping};

#[derive(Debug, Clone)]
pub struct SStatusRequest;

impl Packet for SStatusRequest {
    const NAME: &'static str = "SStatusRequest";

    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2 => 0x00,
    ];

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

#[derive(Debug, Clone)]
pub struct CStatusResponse {
    pub json_response: String,
}

impl Packet for CStatusResponse {
    const NAME: &'static str = "CStatusResponse";

    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2 => 0x00,
    ];

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

#[derive(Debug, Clone)]
pub struct SPingRequest {
    pub payload: i64,
}

impl Packet for SPingRequest {
    const NAME: &'static str = "SPingRequest";

    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2 => 0x01,
    ];

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

#[derive(Debug, Clone)]
pub struct CPingResponse {
    pub payload: i64,
}

impl Packet for CPingResponse {
    const NAME: &'static str = "CPingResponse";

    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: Direction = Direction::Clientbound;
    const IDS: &'static [PacketMapping] = ids![
        V1_7_2 => 0x01,
    ];

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
    use crate::packets::round_trip;
    use crate::registry::build_default_registry;

    #[test]
    fn test_status_request_round_trip() {
        let pkt = SStatusRequest;
        let mut buf = Vec::new();
        pkt.encode(&mut buf, ProtocolVersion::V1_21).unwrap();
        assert!(buf.is_empty());
        let decoded = SStatusRequest::decode(&mut buf.as_slice(), ProtocolVersion::V1_21).unwrap();
        assert_eq!(size_of_val(&decoded), size_of::<SStatusRequest>());
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
