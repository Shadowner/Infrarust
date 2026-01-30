use std::io::{self, Cursor, Read};

pub const LEGACY_PING_BYTE: u8 = 0xFE;

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyPingVariant {
    /// Beta 1.8 – 1.3: Client sends only `0xFE`.
    /// No hostname information available.
    Beta,

    /// 1.4 – 1.5: Client sends `0xFE 0x01`.
    /// No hostname information available.
    V1_4,

    /// 1.6: Client sends `0xFE 0x01 0xFA` followed by MC|PingHost plugin message.
    /// Contains hostname, port, and protocol version.
    V1_6 {
        protocol_version: u8,
        hostname: String,
        port: i32,
    },
}

impl LegacyPingVariant {
    pub fn uses_v1_4_response_format(&self) -> bool {
        matches!(
            self,
            LegacyPingVariant::V1_4 | LegacyPingVariant::V1_6 { .. }
        )
    }

    pub fn hostname(&self) -> Option<&str> {
        match self {
            LegacyPingVariant::V1_6 { hostname, .. } => Some(hostname),
            _ => None,
        }
    }
}

pub fn parse_legacy_ping(data: &[u8]) -> io::Result<LegacyPingVariant> {
    if data.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Empty legacy ping data",
        ));
    }

    let mut cursor = Cursor::new(data);

    let mut first = [0u8; 1];
    cursor.read_exact(&mut first)?;
    if first[0] != LEGACY_PING_BYTE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Expected 0xFE, got 0x{:02X}", first[0]),
        ));
    }

    let mut second = [0u8; 1];
    if cursor.read_exact(&mut second).is_err() {
        // Only 0xFE received — Beta 1.8–1.3
        return Ok(LegacyPingVariant::Beta);
    }

    if second[0] != 0x01 {
        return Ok(LegacyPingVariant::Beta);
    }

    let mut third = [0u8; 1];
    if cursor.read_exact(&mut third).is_err() {
        return Ok(LegacyPingVariant::V1_4);
    }

    if third[0] != 0xFA {
        return Ok(LegacyPingVariant::V1_4);
    }

    let _channel_name = read_legacy_utf16be_string(&mut cursor)?;

    let mut data_len_bytes = [0u8; 2];
    cursor.read_exact(&mut data_len_bytes)?;
    let _data_len = u16::from_be_bytes(data_len_bytes);

    let mut proto_byte = [0u8; 1];
    cursor.read_exact(&mut proto_byte)?;
    let protocol_version = proto_byte[0];

    let hostname = read_legacy_utf16be_string(&mut cursor)?;

    let mut port_bytes = [0u8; 4];
    cursor.read_exact(&mut port_bytes)?;
    let port = i32::from_be_bytes(port_bytes);

    Ok(LegacyPingVariant::V1_6 {
        protocol_version,
        hostname,
        port,
    })
}

fn read_legacy_utf16be_string<R: Read>(reader: &mut R) -> io::Result<String> {
    let mut len_bytes = [0u8; 2];
    reader.read_exact(&mut len_bytes)?;
    let len = u16::from_be_bytes(len_bytes) as usize;

    let mut utf16_bytes = vec![0u8; len * 2];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_beta_ping() {
        let data = [0xFE];
        let result = parse_legacy_ping(&data).unwrap();
        assert_eq!(result, LegacyPingVariant::Beta);
    }

    #[test]
    fn test_parse_v1_4_ping() {
        let data = [0xFE, 0x01];
        let result = parse_legacy_ping(&data).unwrap();
        assert_eq!(result, LegacyPingVariant::V1_4);
    }

    #[test]
    fn test_parse_v1_6_ping() {
        let mut data = Vec::new();
        data.push(0xFE);
        data.push(0x01);
        data.push(0xFA);

        let channel = "MC|PingHost";
        data.extend_from_slice(&(channel.len() as u16).to_be_bytes());
        for ch in channel.encode_utf16() {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        let hostname = "mc.example.com";
        let hostname_utf16_len = hostname.encode_utf16().count();

        let data_len: u16 = (1 + 2 + hostname_utf16_len * 2 + 4) as u16;
        data.extend_from_slice(&data_len.to_be_bytes());

        data.push(74); // 1.6.2

        data.extend_from_slice(&(hostname_utf16_len as u16).to_be_bytes());
        for ch in hostname.encode_utf16() {
            data.extend_from_slice(&ch.to_be_bytes());
        }

        data.extend_from_slice(&25565i32.to_be_bytes());

        let result = parse_legacy_ping(&data).unwrap();
        match result {
            LegacyPingVariant::V1_6 {
                protocol_version,
                hostname: h,
                port,
            } => {
                assert_eq!(protocol_version, 74);
                assert_eq!(h, "mc.example.com");
                assert_eq!(port, 25565);
            }
            other => panic!("Expected V1_6, got {:?}", other),
        }
    }

    #[test]
    fn test_variant_hostname() {
        assert_eq!(LegacyPingVariant::Beta.hostname(), None);
        assert_eq!(LegacyPingVariant::V1_4.hostname(), None);
        assert_eq!(
            LegacyPingVariant::V1_6 {
                protocol_version: 74,
                hostname: "test.com".to_string(),
                port: 25565,
            }
            .hostname(),
            Some("test.com")
        );
    }

    #[test]
    fn test_variant_response_format() {
        assert!(!LegacyPingVariant::Beta.uses_v1_4_response_format());
        assert!(LegacyPingVariant::V1_4.uses_v1_4_response_format());
        assert!(
            LegacyPingVariant::V1_6 {
                protocol_version: 74,
                hostname: "test.com".to_string(),
                port: 25565,
            }
            .uses_v1_4_response_format()
        );
    }
}
