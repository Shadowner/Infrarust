use crate::error::{ProtocolError, ProtocolResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPingVariant {
    Beta,
    V1_4,
    V1_6,
}

#[derive(Debug, Clone)]
pub struct LegacyPingRequest {
    pub variant: LegacyPingVariant,
    pub hostname: Option<String>,
    pub port: Option<i32>,
    pub protocol_version: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct LegacyPingResponse {
    pub protocol_version: i32,
    pub server_version: String,
    pub motd: String,
    pub online_players: i32,
    pub max_players: i32,
}

pub(crate) fn encode_utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

pub(crate) fn decode_utf16be(data: &[u8]) -> ProtocolResult<String> {
    if !data.len().is_multiple_of(2) {
        return Err(ProtocolError::invalid("UTF-16BE data has odd length"));
    }
    let code_units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&code_units).map_err(|_| ProtocolError::invalid("invalid UTF-16BE string"))
}

pub(crate) fn build_kick_packet(payload: &str) -> ProtocolResult<Vec<u8>> {
    let encoded = encode_utf16be(payload);
    let code_unit_count = encoded.len() / 2;
    if code_unit_count > usize::from(u16::MAX) {
        return Err(ProtocolError::too_large(
            usize::from(u16::MAX),
            code_unit_count,
        ));
    }
    let mut out = Vec::with_capacity(1 + 2 + encoded.len());
    out.push(0xFF);
    out.extend_from_slice(&(code_unit_count as u16).to_be_bytes());
    out.extend_from_slice(&encoded);
    Ok(out)
}

impl LegacyPingResponse {
    pub fn build_beta_response(&self) -> ProtocolResult<Vec<u8>> {
        let payload = format!("{}§{}§{}", self.motd, self.online_players, self.max_players);
        build_kick_packet(&payload)
    }

    pub fn build_v1_4_response(&self) -> ProtocolResult<Vec<u8>> {
        let payload = format!(
            "\u{00a7}1\0{}\0{}\0{}\0{}\0{}",
            self.protocol_version,
            self.server_version,
            self.motd,
            self.online_players,
            self.max_players
        );
        build_kick_packet(&payload)
    }
}

pub fn parse_legacy_ping(data: &[u8]) -> ProtocolResult<LegacyPingRequest> {
    if data.is_empty() || data[0] != 0x01 {
        return Ok(LegacyPingRequest {
            variant: LegacyPingVariant::Beta,
            hostname: None,
            port: None,
            protocol_version: None,
        });
    }

    if data.len() < 2 || data[1] != 0xFA {
        return Ok(LegacyPingRequest {
            variant: LegacyPingVariant::V1_4,
            hostname: None,
            port: None,
            protocol_version: None,
        });
    }

    parse_v1_6_ping(&data[2..])
}

fn parse_v1_6_ping(data: &[u8]) -> ProtocolResult<LegacyPingRequest> {
    if data.len() < 2 {
        return Err(ProtocolError::invalid(
            "V1_6 ping: missing channel name length",
        ));
    }

    let channel_len = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let channel_bytes = channel_len * 2;
    let offset = 2 + channel_bytes;

    if data.len() < offset + 2 {
        return Err(ProtocolError::invalid("V1_6 ping: truncated channel name"));
    }

    let _data_len = u16::from_be_bytes([data[offset], data[offset + 1]]);
    let mut pos = offset + 2;

    if pos >= data.len() {
        return Err(ProtocolError::invalid(
            "V1_6 ping: missing protocol version",
        ));
    }
    let protocol_version = data[pos];
    pos += 1;

    if pos + 2 > data.len() {
        return Err(ProtocolError::invalid("V1_6 ping: missing hostname length"));
    }
    let hostname_len = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
    pos += 2;

    let hostname_bytes = hostname_len * 2;
    if pos + hostname_bytes > data.len() {
        return Err(ProtocolError::invalid("V1_6 ping: truncated hostname"));
    }
    let hostname = decode_utf16be(&data[pos..pos + hostname_bytes])?;
    pos += hostname_bytes;

    if pos + 4 > data.len() {
        return Err(ProtocolError::invalid("V1_6 ping: missing port"));
    }
    let port = i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);

    Ok(LegacyPingRequest {
        variant: LegacyPingVariant::V1_6,
        hostname: Some(hostname),
        port: Some(port),
        protocol_version: Some(protocol_version),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::legacy::LegacyDetection;
    use crate::legacy::detect;

    #[test]
    fn test_detect_legacy_ping() {
        assert_eq!(detect(0xFE), LegacyDetection::LegacyPing);
    }

    #[test]
    fn test_detect_legacy_login() {
        assert_eq!(detect(0x02), LegacyDetection::LegacyLogin);
    }

    #[test]
    fn test_detect_modern() {
        assert_eq!(detect(0x00), LegacyDetection::Modern);
        assert_eq!(detect(0x10), LegacyDetection::Modern);
        assert_eq!(detect(0xFF), LegacyDetection::Modern);
    }

    #[test]
    fn test_parse_beta_ping() {
        let req = parse_legacy_ping(&[]).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::Beta);
        assert!(req.hostname.is_none());
        assert!(req.port.is_none());
        assert!(req.protocol_version.is_none());
    }

    #[test]
    fn test_parse_v1_4_ping() {
        let req = parse_legacy_ping(&[0x01]).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_4);
        assert!(req.hostname.is_none());
        assert!(req.port.is_none());
        assert!(req.protocol_version.is_none());
    }

    #[test]
    fn test_parse_v1_4_ping_no_fa() {
        let req = parse_legacy_ping(&[0x01, 0x00]).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_4);
    }

    #[test]
    fn test_parse_v1_6_ping() {
        let mut data = Vec::new();

        let channel = "MC|PingHost";
        let channel_utf16: Vec<u8> = channel.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let channel_code_units = channel.encode_utf16().count() as u16;
        data.extend_from_slice(&channel_code_units.to_be_bytes());
        data.extend_from_slice(&channel_utf16);

        let hostname = "mc.example.com";
        let hostname_utf16: Vec<u8> = hostname.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let hostname_code_units = hostname.encode_utf16().count() as u16;

        let data_length = (1 + 2 + hostname_utf16.len() + 4) as u16;
        data.extend_from_slice(&data_length.to_be_bytes());

        data.push(73);

        data.extend_from_slice(&hostname_code_units.to_be_bytes());
        data.extend_from_slice(&hostname_utf16);

        data.extend_from_slice(&25565_i32.to_be_bytes());

        let req = parse_legacy_ping(&[&[0x01, 0xFA], data.as_slice()].concat()).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_6);
        assert_eq!(req.hostname.as_deref(), Some("mc.example.com"));
        assert_eq!(req.port, Some(25565));
        assert_eq!(req.protocol_version, Some(73));
    }

    #[test]
    fn test_parse_v1_6_hostname_extraction() {
        let mut data = Vec::new();

        let channel = "MC|PingHost";
        let channel_utf16: Vec<u8> = channel.encode_utf16().flat_map(u16::to_be_bytes).collect();
        data.extend_from_slice(&(channel.encode_utf16().count() as u16).to_be_bytes());
        data.extend_from_slice(&channel_utf16);

        let hostname = "play.my-server.net";
        let hostname_utf16: Vec<u8> = hostname.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let data_length = (1 + 2 + hostname_utf16.len() + 4) as u16;
        data.extend_from_slice(&data_length.to_be_bytes());
        data.push(78);
        data.extend_from_slice(&(hostname.encode_utf16().count() as u16).to_be_bytes());
        data.extend_from_slice(&hostname_utf16);
        data.extend_from_slice(&19132_i32.to_be_bytes());

        let req = parse_legacy_ping(&[&[0x01, 0xFA], data.as_slice()].concat()).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_6);
        assert_eq!(req.hostname.as_deref(), Some("play.my-server.net"));
        assert_eq!(req.port, Some(19132));
    }

    #[test]
    fn test_build_beta_response() {
        let resp = LegacyPingResponse {
            protocol_version: 127,
            server_version: "1.21.4".to_string(),
            motd: "Hello".to_string(),
            online_players: 5,
            max_players: 20,
        };
        let bytes = resp.build_beta_response().unwrap();

        assert_eq!(bytes[0], 0xFF);

        let string_len = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        let string_data = &bytes[3..3 + string_len * 2];
        let decoded = decode_utf16be(string_data).unwrap();
        assert_eq!(decoded, "Hello\u{00a7}5\u{00a7}20");
    }

    #[test]
    fn test_build_v1_4_response() {
        let resp = LegacyPingResponse {
            protocol_version: 127,
            server_version: "1.21.4".to_string(),
            motd: "A Minecraft Server".to_string(),
            online_players: 3,
            max_players: 100,
        };
        let bytes = resp.build_v1_4_response().unwrap();

        assert_eq!(bytes[0], 0xFF);

        let string_len = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        let string_data = &bytes[3..3 + string_len * 2];
        let decoded = decode_utf16be(string_data).unwrap();
        assert_eq!(
            decoded,
            "\u{00a7}1\x00127\x001.21.4\x00A Minecraft Server\x003\x00100"
        );
    }

    #[test]
    fn test_response_utf16be_encoding() {
        let resp = LegacyPingResponse {
            protocol_version: 127,
            server_version: "1.21.4".to_string(),
            motd: "Bienvenue \u{00e0} tous!".to_string(),
            online_players: 0,
            max_players: 50,
        };
        let bytes = resp.build_v1_4_response().unwrap();

        assert_eq!(bytes[0], 0xFF);
        let string_len = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        let string_data = &bytes[3..3 + string_len * 2];
        let decoded = decode_utf16be(string_data).unwrap();
        assert!(decoded.contains("Bienvenue \u{00e0} tous!"));
    }

    #[test]
    fn test_parse_v1_6_truncated_after_fa() {
        let data = [0x01, 0xFA];
        let result = parse_legacy_ping(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_v1_6_truncated_mid_hostname() {
        let mut data = vec![0x01, 0xFA];
        data.extend_from_slice(&11u16.to_be_bytes());
        data.extend_from_slice(&[0x00, 0x4D, 0x00, 0x43]);
        let result = parse_legacy_ping(&data);
        assert!(result.is_err());
    }
}
