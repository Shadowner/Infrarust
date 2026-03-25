use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::version::ConnectionState;

/// Shared decode for plugin message packets.
fn decode_plugin_message(r: &mut &[u8]) -> ProtocolResult<(String, Vec<u8>)> {
    let channel = r.read_string()?;
    let data = r.read_remaining()?;
    Ok((channel, data))
}

/// Shared encode for plugin message packets.
fn encode_plugin_message(
    mut w: &mut (impl std::io::Write + ?Sized),
    channel: &str,
    data: &[u8],
) -> ProtocolResult<()> {
    w.write_string(channel)?;
    w.write_all(data)?;
    Ok(())
}

// Plugin message packets carry custom channel data (e.g. `minecraft:brand`, `velocity:player_info`).
// The `data` field contains all remaining bytes after the channel string.
define_twin_packets! {
    clientbound: CPluginMessage,
    serverbound: SPluginMessage,
    state: ConnectionState::Play,
    fields: {
        pub channel: String,
        pub data: Vec<u8>,
    },
    decode(r, _version): {
        let (channel, data) = decode_plugin_message(r)?;
        Ok(Self { channel, data })
    },
    encode(self, w, _version): {
        encode_plugin_message(w, &self.channel, &self.data)
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::Packet;
    use crate::version::ProtocolVersion;

    fn round_trip<P: Packet>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = Vec::new();
        packet.encode(&mut buf, version).unwrap();
        P::decode(&mut buf.as_slice(), version).unwrap()
    }

    #[test]
    fn test_plugin_message_round_trip() {
        let pkt = CPluginMessage {
            channel: "minecraft:brand".to_string(),
            data: vec![0x07, b'I', b'n', b'f', b'r', b'a', b'r', b'u'],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.channel, "minecraft:brand");
        assert_eq!(decoded.data, pkt.data);
    }

    #[test]
    fn test_plugin_message_channel_preserved() {
        let pkt = SPluginMessage {
            channel: "velocity:player_info".to_string(),
            data: vec![1, 2, 3],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.channel, "velocity:player_info");
    }

    #[test]
    fn test_plugin_message_remaining_bytes() {
        // Verify that all bytes after the channel are captured as data
        let pkt = CPluginMessage {
            channel: "test:channel".to_string(),
            data: vec![0xFF; 256],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.data.len(), 256);
        assert!(decoded.data.iter().all(|&b| b == 0xFF));
    }
}
