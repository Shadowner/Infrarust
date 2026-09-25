use crate::error::ProtocolResult;
use crate::packets::play::common::{read_nested_text_component, write_text_component};
use crate::packets::{Packet, PacketMapping};
use crate::version::{ConnectionState, Direction, ProtocolVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CTabListHeaderFooter {
    pub header: Vec<u8>,
    pub footer: Vec<u8>,
}

impl Packet for CTabListHeaderFooter {
    const NAME: &'static str = "CTabListHeaderFooter";

    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: Direction = Direction::Clientbound;
    const ENCODE_ONLY: bool = true;
    const IDS: &'static [PacketMapping] = ids![
        V1_8     => 0x47,
        V1_9     => 0x48,
        V1_9_4   => 0x47,
        V1_12    => 0x49,
        V1_12_1  => 0x4A,
        V1_13    => 0x4E,
        V1_14    => 0x53,
        V1_15    => 0x54,
        V1_16    => 0x53,
        V1_17    => 0x5E,
        V1_18    => 0x5F,
        V1_19    => 0x60,
        V1_19_1  => 0x63,
        V1_19_3  => 0x61,
        V1_19_4  => 0x65,
        V1_20_2  => 0x68,
        V1_20_3  => 0x6A,
        V1_20_5  => 0x6D,
        V1_21_2  => 0x74,
        V1_21_5  => 0x73,
        V1_21_9  => 0x78,
        V26_1    => 0x7A,
    ];

    fn decode(r: &mut &[u8], version: ProtocolVersion) -> ProtocolResult<Self> {
        let header = read_nested_text_component(r, version)?;
        let footer = read_nested_text_component(r, version)?;
        Ok(Self { header, footer })
    }

    fn encode(
        &self,
        w: &mut (impl std::io::Write + ?Sized),
        version: ProtocolVersion,
    ) -> ProtocolResult<()> {
        write_text_component(w, &self.header, version, Self::NAME, "header")?;
        write_text_component(w, &self.footer, version, Self::NAME, "footer")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::packets::round_trip;

    #[test]
    fn json_header_and_footer_round_trip() {
        let packet = CTabListHeaderFooter {
            header: br#"{"text":"top"}"#.to_vec(),
            footer: br#"{"text":"bottom"}"#.to_vec(),
        };
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_8), packet);
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_20_2), packet);
    }

    #[test]
    fn nbt_header_is_split_from_the_footer() {
        let header = vec![0x08, 0x00, 0x03, b't', b'o', b'p'];
        let footer = vec![
            0x0A, 0x08, 0x00, 0x04, b't', b'e', b'x', b't', 0x00, 0x01, b'b', 0x00,
        ];
        let packet = CTabListHeaderFooter { header, footer };
        assert_eq!(round_trip(&packet, ProtocolVersion::V1_20_3), packet);
    }
}
