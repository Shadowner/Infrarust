use bytes::BytesMut;

use crate::protocol::version::Version;

use super::{
    PacketValidation,
    base::{CompressionState, EncryptionState, Packet},
    error::{PacketError, PacketResult},
};

/// Builder for creating Minecraft packets with various configurations.
/// First idea was that the packet itself handle it's encryption and compression,
/// but it's better to have a writer / reader.
/// So i don't use this builder that much and packet system might be rewritten
/// To remove the PacketBuilder.
pub struct PacketBuilder {
    id: Option<i32>,
    data: Option<BytesMut>,
    compression: CompressionState,
    encryption: EncryptionState,
    protocol_version: Version,
}

impl Default for PacketBuilder {
    fn default() -> Self {
        Self {
            id: None,
            data: None,
            compression: CompressionState::Disabled,
            encryption: EncryptionState::Disabled,
            protocol_version: Version::V1_20_2,
        }
    }
}

impl PacketBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: i32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn data(mut self, data: impl Into<BytesMut>) -> Self {
        self.data = Some(data.into());
        self
    }

    pub fn with_compression(mut self, threshold: i32) -> Self {
        self.compression = CompressionState::Enabled { threshold };
        self
    }

    pub fn with_encryption(mut self) -> Self {
        self.encryption = EncryptionState::Enabled {
            encrypted_data: false,
        };
        self
    }

    pub fn protocol_version(mut self, version: Version) -> Self {
        self.protocol_version = version;
        self
    }

    pub fn build(self) -> PacketResult<Packet> {
        let id = self
            .id
            .ok_or_else(|| PacketError::invalid_format("Missing packet ID"))?;
        let data = self.data.unwrap_or_default();

        let packet = Packet {
            id,
            data,
            compression: self.compression,
            encryption: self.encryption,
            protocol_version: self.protocol_version,
        };

        packet.validate_length()?;
        packet.validate_compression()?;
        packet.validate_encryption()?;

        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_builder() {
        let packet = PacketBuilder::new()
            .id(0x00)
            .data(BytesMut::from(&b"test"[..]))
            .build()
            .unwrap();

        assert_eq!(packet.id, 0x00);
        assert_eq!(&packet.data[..], b"test");
    }

    #[test]
    fn test_builder_with_compression() {
        let packet = PacketBuilder::new()
            .id(0x00)
            .with_compression(256)
            .build()
            .unwrap();

        assert_eq!(
            packet.compression,
            CompressionState::Enabled { threshold: 256 }
        );
    }

    #[test]
    fn test_builder_with_encryption() {
        let packet = PacketBuilder::new()
            .id(0x00)
            .with_encryption()
            .build()
            .unwrap();

        assert_eq!(
            packet.encryption,
            EncryptionState::Enabled {
                encrypted_data: false
            }
        );
        assert!(packet.data.is_empty()); // Ensure data is empty for initial build
    }

    #[test]
    fn test_missing_id() {
        let result = PacketBuilder::new().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_protocol_version() {
        let packet = PacketBuilder::new()
            .id(0x00)
            .protocol_version(Version::V1_19_3)
            .build()
            .unwrap();

        assert_eq!(packet.protocol_version, Version::V1_19_3);
    }
}
