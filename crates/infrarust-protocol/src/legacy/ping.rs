use crate::error::{ProtocolError, ProtocolResult};

/// Variant of the legacy ping protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPingVariant {
    /// Beta 1.8 — Just `0xFE`, nothing else.
    Beta,
    /// 1.4–1.5 — `0xFE 0x01`, no plugin message.
    V1_4,
    /// 1.6 — `0xFE 0x01 0xFA` + MC|PingHost with hostname and port.
    V1_6,
}

/// Parsed data from a legacy ping request.
#[derive(Debug, Clone)]
pub struct LegacyPingRequest {
    /// The detected ping variant.
    pub variant: LegacyPingVariant,
    /// Hostname extracted from the MC|PingHost plugin message (`V1_6` only).
    pub hostname: Option<String>,
    /// Port extracted from the plugin message (`V1_6` only).
    pub port: Option<i32>,
    /// Legacy protocol version (`V1_6` only).
    pub protocol_version: Option<u8>,
}

/// Data for constructing a legacy ping response.
#[derive(Debug, Clone)]
pub struct LegacyPingResponse {
    /// Protocol version number (e.g., 127 for legacy compat).
    pub protocol_version: i32,
    /// Server version string (e.g., "1.21.4").
    pub server_version: String,
    /// Message of the day.
    pub motd: String,
    /// Number of online players.
    pub online_players: i32,
    /// Maximum number of players.
    pub max_players: i32,
}

/// Encodes a Rust string into UTF-16 Big-Endian bytes.
fn encode_utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

/// Decodes UTF-16 Big-Endian bytes into a Rust string.
fn decode_utf16be(data: &[u8]) -> ProtocolResult<String> {
    if !data.len().is_multiple_of(2) {
        return Err(ProtocolError::invalid("UTF-16BE data has odd length"));
    }
    let code_units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&code_units).map_err(|_| ProtocolError::invalid("invalid UTF-16BE string"))
}

/// Builds a legacy kick packet from a payload string.
///
/// Format: `0xFF` + `u16 BE` (string length in UTF-16 code units) + UTF-16BE payload.
fn build_kick_packet(payload: &str) -> ProtocolResult<Vec<u8>> {
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
    #[allow(clippy::cast_possible_truncation)] // Checked above that it fits in u16
    out.extend_from_slice(&(code_unit_count as u16).to_be_bytes());
    out.extend_from_slice(&encoded);
    Ok(out)
}

impl LegacyPingResponse {
    /// Builds the kick response for the Beta 1.8 variant.
    ///
    /// Format: `"MOTD§online§max"` — no protocol version or game version.
    pub fn build_beta_response(&self) -> ProtocolResult<Vec<u8>> {
        let payload = format!("{}§{}§{}", self.motd, self.online_players, self.max_players);
        build_kick_packet(&payload)
    }

    /// Builds the kick response for the 1.4+ variants (including 1.6).
    ///
    /// Format: `"§1\0{protocol}\0{version}\0{motd}\0{online}\0{max}"`
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

/// Parses a legacy ping from the bytes AFTER the initial `0xFE`.
///
/// Determines the variant and extracts hostname/port for `V1_6`.
pub fn parse_legacy_ping(data: &[u8]) -> ProtocolResult<LegacyPingRequest> {
    // Beta variant: no additional data, or first byte is not 0x01
    if data.is_empty() || data[0] != 0x01 {
        return Ok(LegacyPingRequest {
            variant: LegacyPingVariant::Beta,
            hostname: None,
            port: None,
            protocol_version: None,
        });
    }

    // V1_4 variant: just 0x01, no plugin message
    if data.len() < 2 || data[1] != 0xFA {
        return Ok(LegacyPingRequest {
            variant: LegacyPingVariant::V1_4,
            hostname: None,
            port: None,
            protocol_version: None,
        });
    }

    // V1_6 variant: 0x01 0xFA + MC|PingHost plugin message
    parse_v1_6_ping(&data[2..])
}

/// Parses the `V1_6` `MC|PingHost` plugin message.
///
/// `data` starts after the `0x01 0xFA` prefix.
///
/// Format:
/// - `u16 BE`: channel name length (in UTF-16 code units)
/// - UTF-16BE: channel name ("MC|PingHost")
/// - `u16 BE`: data length (in bytes)
/// - `u8`: protocol version
/// - `u16 BE`: hostname length (in UTF-16 code units)
/// - UTF-16BE: hostname
/// - `i32 BE`: port
fn parse_v1_6_ping(data: &[u8]) -> ProtocolResult<LegacyPingRequest> {
    if data.len() < 2 {
        return Err(ProtocolError::invalid(
            "V1_6 ping: missing channel name length",
        ));
    }

    let channel_len = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let channel_bytes = channel_len * 2; // UTF-16 = 2 bytes per code unit
    let offset = 2 + channel_bytes;

    if data.len() < offset + 2 {
        return Err(ProtocolError::invalid("V1_6 ping: truncated channel name"));
    }

    // Skip channel name (we don't validate "MC|PingHost" — just extract the data)
    // Read data length (u16 BE)
    let _data_len = u16::from_be_bytes([data[offset], data[offset + 1]]);
    let mut pos = offset + 2;

    // Protocol version (u8)
    if pos >= data.len() {
        return Err(ProtocolError::invalid(
            "V1_6 ping: missing protocol version",
        ));
    }
    let protocol_version = data[pos];
    pos += 1;

    // Hostname length (u16 BE, in UTF-16 code units)
    if pos + 2 > data.len() {
        return Err(ProtocolError::invalid("V1_6 ping: missing hostname length"));
    }
    let hostname_len = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
    pos += 2;

    // Hostname (UTF-16BE)
    let hostname_bytes = hostname_len * 2;
    if pos + hostname_bytes > data.len() {
        return Err(ProtocolError::invalid("V1_6 ping: truncated hostname"));
    }
    let hostname = decode_utf16be(&data[pos..pos + hostname_bytes])?;
    pos += hostname_bytes;

    // Port (i32 BE)
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
        // 0x01 followed by something other than 0xFA
        let req = parse_legacy_ping(&[0x01, 0x00]).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_4);
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // Test values are known to fit
    fn test_parse_v1_6_ping() {
        // Build a complete MC|PingHost plugin message
        let mut data = Vec::new();

        // Channel name "MC|PingHost" in UTF-16BE
        let channel = "MC|PingHost";
        let channel_utf16: Vec<u8> = channel.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let channel_code_units = channel.encode_utf16().count() as u16;
        data.extend_from_slice(&channel_code_units.to_be_bytes());
        data.extend_from_slice(&channel_utf16);

        // Data section
        let hostname = "mc.example.com";
        let hostname_utf16: Vec<u8> = hostname.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let hostname_code_units = hostname.encode_utf16().count() as u16;

        // data_length = 1 (protocol_version) + 2 (hostname_len) + hostname_bytes + 4 (port)
        let data_length = (1 + 2 + hostname_utf16.len() + 4) as u16;
        data.extend_from_slice(&data_length.to_be_bytes());

        // Protocol version
        data.push(73); // 1.6.1

        // Hostname
        data.extend_from_slice(&hostname_code_units.to_be_bytes());
        data.extend_from_slice(&hostname_utf16);

        // Port
        data.extend_from_slice(&25565_i32.to_be_bytes());

        let req = parse_legacy_ping(&[&[0x01, 0xFA], data.as_slice()].concat()).unwrap();
        assert_eq!(req.variant, LegacyPingVariant::V1_6);
        assert_eq!(req.hostname.as_deref(), Some("mc.example.com"));
        assert_eq!(req.port, Some(25565));
        assert_eq!(req.protocol_version, Some(73));
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // Test values are known to fit
    fn test_parse_v1_6_hostname_extraction() {
        // Test with a different hostname containing subdomain
        let mut data = Vec::new();

        let channel = "MC|PingHost";
        let channel_utf16: Vec<u8> = channel.encode_utf16().flat_map(u16::to_be_bytes).collect();
        data.extend_from_slice(&(channel.encode_utf16().count() as u16).to_be_bytes());
        data.extend_from_slice(&channel_utf16);

        let hostname = "play.my-server.net";
        let hostname_utf16: Vec<u8> = hostname.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let data_length = (1 + 2 + hostname_utf16.len() + 4) as u16;
        data.extend_from_slice(&data_length.to_be_bytes());
        data.push(78); // protocol version
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

        // First byte: 0xFF
        assert_eq!(bytes[0], 0xFF);

        // Decode the UTF-16BE string
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
        // Test with non-ASCII characters (accented characters)
        let resp = LegacyPingResponse {
            protocol_version: 127,
            server_version: "1.21.4".to_string(),
            motd: "Bienvenue \u{00e0} tous!".to_string(), // "Bienvenue à tous!"
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
        // 0x01 0xFA but nothing after → must return an error, not panic
        let data = [0x01, 0xFA];
        let result = parse_legacy_ping(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_v1_6_truncated_mid_hostname() {
        // Beginning of a valid V1_6 but truncated in the middle of the channel name
        let mut data = vec![0x01, 0xFA];
        // u16 BE channel name length (11 code units for "MC|PingHost")
        data.extend_from_slice(&11u16.to_be_bytes());
        // Only a few bytes of the channel name, not enough
        data.extend_from_slice(&[0x00, 0x4D, 0x00, 0x43]);
        let result = parse_legacy_ping(&data);
        assert!(result.is_err());
    }
}
