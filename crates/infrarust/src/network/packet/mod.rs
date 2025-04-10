mod base;
mod builder;
mod error;
pub mod io;

pub use base::{CompressionState, EncryptionState, Packet, PacketCodec, PacketValidation};
pub use builder::PacketBuilder;
pub use error::{PacketError, PacketResult};
pub use io::{PacketReader, PacketWriter};

// Public packet system constants (CF : https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol)
pub const MAX_PACKET_LENGTH: usize = 2097151; // 2^21 - 1 (3-byte VarInt max)
pub const MAX_PACKET_DATA_LENGTH: usize = 0x200000; // 2MB
pub const MAX_UNCOMPRESSED_LENGTH: usize = 8388608; // 2^23

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn test_packet_creation_workflow() {
        let packet = PacketBuilder::new()
            .id(0x00)
            .data(BytesMut::from(&b"test"[..]))
            .with_compression(256)
            .build()
            .unwrap();

        assert_eq!(packet.id, 0x00);
        assert_eq!(&packet.data[..], b"test");
        assert!(matches!(
            packet.compression,
            CompressionState::Enabled { threshold: 256 }
        ));
    }
}
