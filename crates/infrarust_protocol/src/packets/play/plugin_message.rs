use crate::codec::{McBufReadExt, McBufWriteExt};
use crate::error::ProtocolResult;
use crate::version::ConnectionState;

fn decode_plugin_message(r: &mut &[u8]) -> ProtocolResult<(String, Vec<u8>)> {
    let channel = r.read_string()?;
    let data = r.read_remaining()?;
    Ok((channel, data))
}

fn encode_plugin_message(
    mut w: &mut (impl std::io::Write + ?Sized),
    channel: &str,
    data: &[u8],
) -> ProtocolResult<()> {
    w.write_string(channel)?;
    w.write_all(data)?;
    Ok(())
}

define_twin_packets! {
    clientbound: CPluginMessage,
    serverbound: SPluginMessage,
    state: ConnectionState::Play,
    clientbound_ids: ids![
        V1_7_2  => 0x3F,
        V1_9    => 0x18,
        V1_13   => 0x19,
        V1_14   => 0x18,
        V1_15   => 0x19,
        V1_16   => 0x18,
        V1_16_2 => 0x17,
        V1_17   => 0x18,
        V1_19   => 0x15,
        V1_19_1 => 0x16,
        V1_19_3 => 0x15,
        V1_19_4 => 0x17,
        V1_20_2 => 0x18,
        V1_20_5 => 0x19,
        V1_21_5 => 0x18,
    ],
    serverbound_ids: ids![
        V1_7_2  => 0x17,
        V1_9    => 0x09,
        V1_12   => 0x0A,
        V1_12_1 => 0x09,
        V1_13   => 0x0A,
        V1_14   => 0x0B,
        V1_17   => 0x0A,
        V1_19   => 0x0C,
        V1_19_1 => 0x0D,
        V1_19_3 => 0x0C,
        V1_19_4 => 0x0D,
        V1_20_2 => 0x0F,
        V1_20_3 => 0x10,
        V1_20_5 => 0x12,
        V1_21_2 => 0x14,
        V1_21_6 => 0x15,
        V26_1   => 0x16,
    ],
    encode_only: true,
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
    use crate::packets::round_trip;
    use crate::version::ProtocolVersion;

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
        let pkt = CPluginMessage {
            channel: "test:channel".to_string(),
            data: vec![0xFF; 256],
        };
        let decoded = round_trip(&pkt, ProtocolVersion::V1_21);
        assert_eq!(decoded.data.len(), 256);
        assert!(decoded.data.iter().all(|&b| b == 0xFF));
    }
}
