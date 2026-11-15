use crate::error::{ProtocolError, ProtocolResult};
use crate::packets::login::check_login_name;

use super::ping::{build_kick_packet, decode_utf16be};
use super::{MAX_HOSTNAME_CHARS, MAX_USERNAME_CHARS};

#[derive(Debug, Clone)]
pub struct LegacyHandshakeRequest {
    pub protocol_version: u8,
    pub username: String,
    pub hostname: String,
    pub port: i32,
}

fn read_string16(
    data: &[u8],
    pos: usize,
    max_chars: usize,
    context: &str,
) -> ProtocolResult<(String, usize)> {
    if pos + 2 > data.len() {
        return Err(ProtocolError::invalid(format!(
            "legacy handshake: missing {context} length"
        )));
    }
    let char_count = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
    if char_count > max_chars {
        return Err(ProtocolError::too_large(max_chars, char_count));
    }
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

pub fn parse_legacy_handshake(data: &[u8]) -> ProtocolResult<LegacyHandshakeRequest> {
    if data.is_empty() {
        return Err(ProtocolError::invalid(
            "legacy handshake: missing format byte",
        ));
    }

    if data[0] == 0x00 {
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
        parse_pre_1_3_connection_string(&connection_string)
    } else {
        let protocol_version = data[0];
        let pos = 1;

        let (username, pos) = read_string16(data, pos, MAX_USERNAME_CHARS, "username")?;
        check_login_name(&username)?;
        let (hostname, pos) = read_string16(data, pos, MAX_HOSTNAME_CHARS, "hostname")?;

        if pos + 4 > data.len() {
            return Err(ProtocolError::invalid("legacy handshake: missing port"));
        }
        let port = i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);

        Ok(LegacyHandshakeRequest {
            protocol_version,
            username,
            hostname,
            port,
        })
    }
}

fn parse_pre_1_3_connection_string(s: &str) -> ProtocolResult<LegacyHandshakeRequest> {
    let (username, hostname, port) = match s.split_once(';') {
        Some((username, host_port)) => match host_port.rsplit_once(':') {
            Some((hostname, port_str)) => {
                let port = port_str.parse::<u16>().map_err(|_| {
                    ProtocolError::invalid(format!("legacy handshake: invalid port {port_str:?}"))
                })?;
                (username, hostname, i32::from(port))
            }
            None => (username, host_port, 25565),
        },
        None => (s, "", 25565),
    };
    check_length(username, MAX_USERNAME_CHARS)?;
    check_login_name(username)?;
    check_length(hostname, MAX_HOSTNAME_CHARS)?;
    Ok(LegacyHandshakeRequest {
        protocol_version: 0,
        username: username.to_string(),
        hostname: hostname.to_string(),
        port,
    })
}

fn check_length(s: &str, max_chars: usize) -> ProtocolResult<()> {
    let chars = s.encode_utf16().count();
    if chars > max_chars {
        return Err(ProtocolError::too_large(max_chars, chars));
    }
    Ok(())
}

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
    fn test_parse_pre_1_3_malformed_port_rejected() {
        let data = build_pre_1_3_packet("Steve;mc.example.com:notaport");
        let err = parse_legacy_handshake(&data).unwrap_err();
        assert!(matches!(err, ProtocolError::Invalid { .. }));
    }

    #[test]
    fn test_parse_pre_1_3_out_of_range_port_rejected() {
        let data = build_pre_1_3_packet("Steve;mc.example.com:99999");
        assert!(parse_legacy_handshake(&data).is_err());
    }

    #[test]
    fn test_parse_pre_1_3_oversized_fields_rejected() {
        let long_name = "a".repeat(MAX_USERNAME_CHARS + 1);
        for connection_string in [
            long_name.clone(),
            format!("{long_name};mc.example.com:25565"),
            format!("{long_name};mc.example.com"),
        ] {
            let data = build_pre_1_3_packet(&connection_string);
            assert!(
                matches!(
                    parse_legacy_handshake(&data),
                    Err(ProtocolError::TooLarge { .. })
                ),
                "{connection_string}"
            );
        }
    }

    #[test]
    fn test_parse_longest_fields_accepted() {
        let name = "a".repeat(MAX_USERNAME_CHARS);
        let host = "b".repeat(MAX_HOSTNAME_CHARS);
        let data = build_handshake_packet(78, &name, &host, 25565);
        let req = parse_legacy_handshake(&data).unwrap();
        assert_eq!(req.username, name);
        assert_eq!(req.hostname, host);
    }

    #[test]
    fn test_parse_oversized_fields_rejected_whatever_follows() {
        let name = "a".repeat(MAX_USERNAME_CHARS + 1);
        let host = "b".repeat(MAX_HOSTNAME_CHARS + 1);
        for data in [
            build_handshake_packet(78, &name, "localhost", 25565),
            build_handshake_packet(78, "Steve", &host, 25565),
            [&[78][..], &u16::MAX.to_be_bytes()[..]].concat(),
        ] {
            assert!(matches!(
                parse_legacy_handshake(&data),
                Err(ProtocolError::TooLarge { .. })
            ));
        }
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
