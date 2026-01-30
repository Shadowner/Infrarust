use std::io::{self, Cursor, Read};

pub const LEGACY_HANDSHAKE_PACKET_ID: u8 = 0x02;

/// [0x02] [byte: protocol_version] [short+UTF-16BE: username] [short+UTF-16BE: hostname] [i32: port]
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyHandshake {
    pub protocol_version: u8,
    pub username: String,
    pub hostname: String,
    pub port: i32,
}

/// The caller has already determined the first byte is `0x02`.
pub fn parse_legacy_handshake(data: &[u8]) -> io::Result<LegacyHandshake> {
    if data.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Empty legacy handshake data",
        ));
    }

    let mut cursor = Cursor::new(data);

    // Consume the packet ID byte (0x02)
    let mut packet_id = [0u8; 1];
    cursor.read_exact(&mut packet_id)?;
    if packet_id[0] != LEGACY_HANDSHAKE_PACKET_ID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Expected packet ID 0x02, got 0x{:02X}", packet_id[0]),
        ));
    }

    let mut format_byte = [0u8; 1];
    cursor.read_exact(&mut format_byte)?;

    if format_byte[0] == 0x00 {
        let mut low_byte = [0u8; 1];
        cursor.read_exact(&mut low_byte)?;
        let str_len = u16::from_be_bytes([0x00, low_byte[0]]) as usize;

        let mut utf16_bytes = vec![0u8; str_len * 2];
        cursor.read_exact(&mut utf16_bytes)?;

        let utf16_units: Vec<u16> = utf16_bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();

        let connection_string = String::from_utf16(&utf16_units).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid UTF-16BE connection string: {}", e),
            )
        })?;

        Ok(parse_pre_1_3_connection_string(&connection_string))
    } else {
        let protocol_version = format_byte[0];

        let username = read_legacy_string(&mut cursor)?;
        let hostname = read_legacy_string(&mut cursor)?;

        let mut port_bytes = [0u8; 4];
        cursor.read_exact(&mut port_bytes)?;
        let port = i32::from_be_bytes(port_bytes);

        Ok(LegacyHandshake {
            protocol_version,
            username,
            hostname,
            port,
        })
    }
}

fn parse_pre_1_3_connection_string(s: &str) -> LegacyHandshake {
    if let Some((username, host_port)) = s.split_once(';') {
        if let Some((hostname, port_str)) = host_port.rsplit_once(':') {
            let port = port_str.parse::<i32>().unwrap_or(25565);
            LegacyHandshake {
                protocol_version: 0,
                username: username.to_string(),
                hostname: hostname.to_string(),
                port,
            }
        } else {
            // Semicolon but no colon — hostname without port
            LegacyHandshake {
                protocol_version: 0,
                username: username.to_string(),
                hostname: host_port.to_string(),
                port: 25565,
            }
        }
    } else {
        // No semicolon — just a username (direct connect, no hostname)
        LegacyHandshake {
            protocol_version: 0,
            username: s.to_string(),
            hostname: String::new(),
            port: 25565,
        }
    }
}

pub fn read_legacy_string<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut len_bytes = [0u8; 2];
    reader.read_exact(&mut len_bytes)?;
    let char_count = u16::from_be_bytes(len_bytes) as usize;

    let mut utf16_bytes = vec![0u8; char_count * 2];
    reader.read_exact(&mut utf16_bytes)?;

    let utf16_units: Vec<u16> = utf16_bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();

    String::from_utf16(&utf16_units).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid UTF-16BE string: {}", e),
        )
    })
}

/// Calculate the total byte size of a legacy handshake packet.
/// Returns `None` if the data is incomplete.
pub fn legacy_handshake_byte_length(data: &[u8]) -> Option<usize> {
    if data.len() < 2 {
        return None;
    }

    if data[1] == 0x00 {
        if data.len() < 3 {
            return None;
        }
        let str_len = u16::from_be_bytes([data[1], data[2]]) as usize;
        Some(1 + 2 + str_len * 2) // packet_id + short + string_data
    } else {
        let mut pos = 2; // Skip packet_id (1) + protocol_version (1)

        if data.len() < pos + 2 {
            return None;
        }
        let username_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + username_len * 2;

        if data.len() < pos + 2 {
            return None;
        }
        let hostname_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2 + hostname_len * 2;

        // Port: 4 bytes
        pos += 4;

        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_legacy_handshake(proto: u8, username: &str, hostname: &str, port: i32) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(LEGACY_HANDSHAKE_PACKET_ID);
        data.push(proto);

        // Username
        let username_utf16: Vec<u16> = username.encode_utf16().collect();
        data.extend_from_slice(&(username_utf16.len() as u16).to_be_bytes());
        for ch in &username_utf16 {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        // Hostname
        let hostname_utf16: Vec<u16> = hostname.encode_utf16().collect();
        data.extend_from_slice(&(hostname_utf16.len() as u16).to_be_bytes());
        for ch in &hostname_utf16 {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        // Port
        data.extend_from_slice(&port.to_be_bytes());

        data
    }

    #[test]
    fn test_parse_legacy_handshake() {
        let data = build_legacy_handshake(74, "Steve", "mc.example.com", 25565);
        let result = parse_legacy_handshake(&data).unwrap();
        assert_eq!(result.protocol_version, 74);
        assert_eq!(result.username, "Steve");
        assert_eq!(result.hostname, "mc.example.com");
        assert_eq!(result.port, 25565);
    }

    #[test]
    fn test_parse_legacy_handshake_unicode() {
        let data = build_legacy_handshake(61, "Héro_ïne", "serveur.exemple.fr", 25565);
        let result = parse_legacy_handshake(&data).unwrap();
        assert_eq!(result.username, "Héro_ïne");
        assert_eq!(result.hostname, "serveur.exemple.fr");
    }

    #[test]
    fn test_legacy_handshake_byte_length() {
        let data = build_legacy_handshake(74, "Steve", "mc.example.com", 25565);
        let len = legacy_handshake_byte_length(&data).unwrap();
        assert_eq!(len, data.len());
    }

    #[test]
    fn test_invalid_packet_id() {
        let data = [0x03, 74]; // Wrong packet ID
        let result = parse_legacy_handshake(&data);
        assert!(result.is_err());
    }

    fn build_pre_1_3_handshake(connection_string: &str) -> Vec<u8> {
        let mut data = Vec::new();
        data.push(LEGACY_HANDSHAKE_PACKET_ID);

        let utf16: Vec<u16> = connection_string.encode_utf16().collect();
        data.extend_from_slice(&(utf16.len() as u16).to_be_bytes());
        for ch in &utf16 {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        data
    }

    #[test]
    fn test_parse_pre_1_3_handshake() {
        let data = build_pre_1_3_handshake("Steve;mc.example.com:25565");
        let result = parse_legacy_handshake(&data).unwrap();
        assert_eq!(result.protocol_version, 0);
        assert_eq!(result.username, "Steve");
        assert_eq!(result.hostname, "mc.example.com");
        assert_eq!(result.port, 25565);
    }

    #[test]
    fn test_parse_pre_1_3_handshake_no_hostname() {
        let data = build_pre_1_3_handshake("Steve");
        let result = parse_legacy_handshake(&data).unwrap();
        assert_eq!(result.protocol_version, 0);
        assert_eq!(result.username, "Steve");
        assert_eq!(result.hostname, "");
        assert_eq!(result.port, 25565);
    }

    #[test]
    fn test_parse_pre_1_3_handshake_custom_port() {
        let data = build_pre_1_3_handshake("Player1;survival.server.net:25570");
        let result = parse_legacy_handshake(&data).unwrap();
        assert_eq!(result.username, "Player1");
        assert_eq!(result.hostname, "survival.server.net");
        assert_eq!(result.port, 25570);
    }

    #[test]
    fn test_pre_1_3_handshake_byte_length() {
        let data = build_pre_1_3_handshake("Steve;mc.example.com:25565");
        let len = legacy_handshake_byte_length(&data).unwrap();
        assert_eq!(len, data.len());
    }

    #[test]
    fn test_1_3_plus_handshake_byte_length() {
        let data = build_legacy_handshake(74, "Steve", "mc.example.com", 25565);
        let len = legacy_handshake_byte_length(&data).unwrap();
        assert_eq!(len, data.len());
    }
}
