use bytes::BytesMut;

use infrarust_protocol::{
    packet::{CompressionState, EncryptionState, PacketError, PacketValidation, Result},
    version::Version,
};

use super::base::Packet;

/// Builder for creating Minecraft packets with various configurations.
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

    pub fn build(self) -> Result<Packet> {
        let id = self
            .id
            .ok_or_else(|| PacketError::InvalidFormat("Missing packet ID".to_string()))?;
        let data = self.data.unwrap_or_default();

        let packet = Packet {
            id,
            data,
            compression: self.compression,
            encryption: self.encryption,
            protocol_version: self.protocol_version,
        };

        packet.validate()?;

        Ok(packet)
    }
}
