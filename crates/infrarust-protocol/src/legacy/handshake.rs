//! Pre-Netty handshake (0x02) parsing for Minecraft 1.6 and earlier.
//!
//! Two formats are supported:
//! - **Pre-1.3**: `0x02` + `0x00` + `u16 BE string_len` (low byte only, high was `0x00`) + UTF-16BE `"username;hostname:port"`
//! - **1.3+**: `0x02` + `u8 protocol_version` + `string16 username` + `string16 hostname` + `i32 port`

use crate::error::{ProtocolError, ProtocolResult};

use super::ping::{build_kick_packet, decode_utf16be};

#[derive(Debug, Clone)]
pub struct LegacyHandshakeRequest {
    pub protocol_version: u8,
    pub username: String,
    pub hostname: String,
    pub port: i32,
}

fn read_string16(data: &[u8], pos: usize, context: &str) -> ProtocolResult<(String, usize)> {
    if pos + 2 > data.len() {
        return Err(ProtocolError::invalid(format!(
            "legacy handshake: missing {context} length"
        )));
    }
    let char_count = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
    let byte_count = char_count * 2;
    let start = pos + 2;
    if start + byte_count > data.len() {
        return Err(ProtocolError::invalid(format!(
            "legacy handshake: truncated {context}"
        )));
    }
    let s = decode_utf16be(&data[start..start + byte_count])?;
    Ok((s, start + byte_count))
}

/// # Errors
/// Returns an error if the packet is truncated or malformed.
pub fn parse_legacy_handshake(data: &[u8]) -> ProtocolResult<LegacyHandshakeRequest> {
    if data.is_empty() {
        return Err(ProtocolError::invalid(
            "legacy handshake: missing format byte",
        ));
    }

    if data[0] == 0x00 {
        // Pre-1.3 format: [0x00] [low_byte_of_string_len] [UTF-16BE connection string]
        if data.len() < 2 {
            return Err(ProtocolError::invalid(
                "legacy handshake: missing string length",
            ));
        }
        let str_len = usize::from(u16::from_be_bytes([0x00, data[1]]));
        let byte_count = str_len * 2;
        let start = 2;
        if start + byte_count > data.len() {
            return Err(ProtocolError::invalid(
                "legacy handshake: truncated connection string",
            ));
        }
        let connection_string = decode_utf16be(&data[start..start + byte_count])?;
        Ok(parse_pre_1_3_connection_string(&connection_string))
    } else {
        // 1.3+ format: [protocol_version] [string16 username] [string16 hostname] [i32 port]
        let protocol_version = data[0];
        let pos = 1;

        let (username, pos) = read_string16(data, pos, "username")?;
        let (hostname, pos) = read_string16(data, pos, "hostname")?;

        if pos + 4 > data.len() {
            return Err(ProtocolError::invalid("legacy handshake: missing port"));
        }
        let port =
            i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);

        Ok(LegacyHandshakeRequest {
            protocol_version,
            username,
            hostname,
            port,
        })
    }
}

fn parse_pre_1_3_connection_string(s: &str) -> LegacyHandshakeRequest {
    if let Some((username, host_port)) = s.split_once(';') {
        if let Some((hostname, port_str)) = host_port.rsplit_once(':') {
            let port = port_str.parse::<i32>().unwrap_or(25565);
            LegacyHandshakeRequest {
                protocol_version: 0,
                username: username.to_string(),
                hostname: hostname.to_string(),
                port,
            }
        } else {
            LegacyHandshakeRequest {
                protocol_version: 0,
                username: username.to_string(),
                hostname: host_port.to_string(),
                port: 25565,
            }
        }
    } else {
        LegacyHandshakeRequest {
            protocol_version: 0,
            username: s.to_string(),
            hostname: String::new(),
            port: 25565,
        }
    }
}

/// # Errors
/// Returns an error if the reason string is too long for the packet format.
pub fn build_legacy_kick(reason: &str) -> ProtocolResult<Vec<u8>> {
    build_kick_packet(reason)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn encode_string16(s: &str) -> Vec<u8> {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let mut out = Vec::new();
        out.extend_from_slice(&(utf16.len() as u16).to_be_bytes());
        for code_unit in &utf16 {
            out.extend_from_slice(&code_unit.to_be_bytes());
        }
        out
    }

    fn build_handshake_packet(protocol: u8, username: &str, hostname: &str, port: i32) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(protocol);
        data.extend_from_slice(&encode_string16(username));
        data.extend_from_slice(&encode_string16(hostname));
        data.extend_from_slice(&port.to_be_bytes());
        data
    }

    fn build_pre_1_3_packet(connection_string: &str) -> Vec<u8> {
        let utf16: Vec<u16> = connection_string.encode_utf16().collect();
        let mut data = Vec::new();
        // Format byte 0x00 (pre-1.3)
        data.push(0x00);
        data.push(utf16.len() as u8);
        for code_unit in &utf16 {
            data.extend_from_slice(&code_unit.to_be_bytes());
        }
        data
    }

    #[test]
    fn test_parse_valid_handshake() {
        let data = build_handshake_packet(78, "Steve", "mc.example.com", 25565);
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.protocol_version, 78);
        assert_eq!(req.username, "Steve");
        assert_eq!(req.hostname, "mc.example.com");
        assert_eq!(req.port, 25565);
    }

    #[test]
    fn test_parse_handshake_different_versions() {
        let data = build_handshake_packet(74, "Player", "localhost", 25565);
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.protocol_version, 74);
        assert_eq!(req.username, "Player");
        assert_eq!(req.hostname, "localhost");
    }

    #[test]
    fn test_parse_pre_1_3_handshake() {
        let data = build_pre_1_3_packet("Steve;mc.example.com:25565");
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.protocol_version, 0);
        assert_eq!(req.username, "Steve");
        assert_eq!(req.hostname, "mc.example.com");
        assert_eq!(req.port, 25565);
    }

    #[test]
    fn test_parse_pre_1_3_no_hostname() {
        let data = build_pre_1_3_packet("Steve");
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.protocol_version, 0);
        assert_eq!(req.username, "Steve");
        assert_eq!(req.hostname, "");
        assert_eq!(req.port, 25565);
    }

    #[test]
    fn test_parse_pre_1_3_no_port() {
        let data = build_pre_1_3_packet("Player1;survival.server.net");
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.username, "Player1");
        assert_eq!(req.hostname, "survival.server.net");
        assert_eq!(req.port, 25565);
    }

    #[test]
    fn test_parse_empty_data() {
        assert!(parse_legacy_handshake(&[]).is_err());
    }

    #[test]
    fn test_parse_truncated_username() {
        assert!(parse_legacy_handshake(&[78]).is_err());
    }

    #[test]
    fn test_parse_truncated_hostname() {
        let mut data = Vec::new();
        data.push(78);
        data.extend_from_slice(&encode_string16("Steve"));
        assert!(parse_legacy_handshake(&data).is_err());
    }

    #[test]
    fn test_parse_missing_port() {
        let mut data = Vec::new();
        data.push(78);
        data.extend_from_slice(&encode_string16("Steve"));
        data.extend_from_slice(&encode_string16("localhost"));
        assert!(parse_legacy_handshake(&data).is_err());
    }

    #[test]
    fn test_build_legacy_kick() {
        let kick = build_legacy_kick("Server is full").unwrap();
        assert_eq!(kick[0], 0xFF);
        let len = u16::from_be_bytes([kick[1], kick[2]]);
        assert_eq!(len as usize, "Server is full".encode_utf16().count());
    }
}
