/// The payload string format differs between protocol versions:
///
/// - **Beta 1.8-1.3**: `"motd§online§max"`
/// - **1.4+**: `"§1\0protocol\0version_name\0motd\0online\0max"`
pub const LEGACY_KICK_PACKET_ID: u8 = 0xFF;

pub fn build_legacy_kick_beta(motd: &str, online_players: i32, max_players: i32) -> Vec<u8> {
    let payload = format!("{}§{}§{}", motd, online_players, max_players);
    encode_legacy_kick_packet(&payload)
}

pub fn build_legacy_kick_v1_4(
    protocol_version: i32,
    version_name: &str,
    motd: &str,
    online_players: i32,
    max_players: i32,
) -> Vec<u8> {
    let payload = format!(
        "\u{00A7}1\0{}\0{}\0{}\0{}\0{}",
        protocol_version, version_name, motd, online_players, max_players
    );
    encode_legacy_kick_packet(&payload)
}

fn encode_legacy_kick_packet(payload: &str) -> Vec<u8> {
    let utf16_units: Vec<u16> = payload.encode_utf16().collect();
    let len = utf16_units.len() as u16;

    let mut packet = Vec::with_capacity(1 + 2 + utf16_units.len() * 2);
    packet.push(LEGACY_KICK_PACKET_ID);
    packet.extend_from_slice(&len.to_be_bytes());
    for unit in &utf16_units {
        packet.extend_from_slice(&unit.to_be_bytes());
    }

    packet
}

pub fn encode_utf16be(s: &str) -> Vec<u8> {
    let utf16_units: Vec<u16> = s.encode_utf16().collect();
    let mut bytes = Vec::with_capacity(utf16_units.len() * 2);
    for unit in &utf16_units {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

pub fn decode_utf16be(data: &[u8]) -> std::io::Result<String> {
    if !data.len().is_multiple_of(2) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "UTF-16BE data must have even length",
        ));
    }

    let utf16_units: Vec<u16> = data
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();

    String::from_utf16(&utf16_units).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Invalid UTF-16BE data: {}", e),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_beta_kick() {
        let packet = build_legacy_kick_beta("A Minecraft Server", 0, 20);

        assert_eq!(packet[0], LEGACY_KICK_PACKET_ID);

        let str_len = u16::from_be_bytes([packet[1], packet[2]]) as usize;
        let string_data = &packet[3..3 + str_len * 2];
        let decoded = decode_utf16be(string_data).unwrap();

        assert_eq!(decoded, "A Minecraft Server§0§20");
    }

    #[test]
    fn test_build_v1_4_kick() {
        let packet = build_legacy_kick_v1_4(47, "1.4.2", "A Minecraft Server", 0, 20);

        assert_eq!(packet[0], LEGACY_KICK_PACKET_ID);

        let str_len = u16::from_be_bytes([packet[1], packet[2]]) as usize;
        let string_data = &packet[3..3 + str_len * 2];
        let decoded = decode_utf16be(string_data).unwrap();

        let parts: Vec<&str> = decoded.split('\0').collect();
        assert_eq!(parts[0], "\u{00A7}1"); // §1
        assert_eq!(parts[1], "47"); // protocol version
        assert_eq!(parts[2], "1.4.2"); // version name
        assert_eq!(parts[3], "A Minecraft Server"); // motd
        assert_eq!(parts[4], "0"); // online
        assert_eq!(parts[5], "20"); // max
    }

    #[test]
    fn test_encode_decode_utf16be() {
        let original = "Hello, Minecraft! §";
        let encoded = encode_utf16be(original);
        let decoded = decode_utf16be(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_utf16be_unicode() {
        let original = "§1\0test\0data";
        let encoded = encode_utf16be(original);
        let decoded = decode_utf16be(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_decode_utf16be_odd_length() {
        let result = decode_utf16be(&[0x00, 0x41, 0x00]);
        assert!(result.is_err());
    }
}
