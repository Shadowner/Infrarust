//! Mock implementations of packet interfaces for testing and examples

use bytes::BytesMut;
use std::io::Cursor;

use super::{
    CompressionControl, CompressionState, EncryptionControl, EncryptionState, PacketCodec,
    PacketDataAccess, PacketError, PacketFactory, PacketSerialization, PacketValidation, Result,
};
use crate::types::{ProtocolRead, ProtocolWrite, VarInt, WriteToBytes};
use crate::version::Version;

/// A simple mock packet implementation for testing
#[derive(Debug, Clone)]
pub struct MockPacket {
    pub id: i32,
    pub data: BytesMut,
    pub protocol_version: Version,
    pub compression: CompressionState,
    pub encryption: EncryptionState,
}

impl MockPacket {
    pub fn new(id: i32) -> Self {
        Self {
            id,
            data: BytesMut::new(),
            protocol_version: Version::V1_20_2,
            compression: CompressionState::Disabled,
            encryption: EncryptionState::Disabled,
        }
    }
}

impl PacketDataAccess for MockPacket {
    fn id(&self) -> i32 {
        self.id
    }

    fn data(&self) -> &[u8] {
        &self.data
    }

    fn protocol_version(&self) -> Version {
        self.protocol_version
    }

    fn set_protocol_version(&mut self, version: Version) {
        self.protocol_version = version;
    }
}

impl CompressionControl for MockPacket {
    fn compression_state(&self) -> CompressionState {
        self.compression
    }

    fn enable_compression(&mut self, threshold: i32) {
        self.compression = CompressionState::Enabled { threshold };
    }

    fn disable_compression(&mut self) {
        self.compression = CompressionState::Disabled;
    }

    fn is_compressing(&self) -> bool {
        matches!(self.compression, CompressionState::Enabled { .. })
    }
}

impl EncryptionControl for MockPacket {
    fn encryption_state(&self) -> EncryptionState {
        self.encryption.clone()
    }

    fn enable_encryption(&mut self) {
        self.encryption = EncryptionState::Enabled {
            encrypted_data: false,
        };
    }

    fn disable_encryption(&mut self) {
        self.encryption = EncryptionState::Disabled;
    }

    fn mark_as_encrypted(&mut self) {
        if let EncryptionState::Enabled {
            ref mut encrypted_data,
        } = self.encryption
        {
            *encrypted_data = true;
        }
    }

    fn is_encrypted(&self) -> bool {
        matches!(self.encryption, EncryptionState::Enabled { .. })
    }
}

impl PacketValidation for MockPacket {
    fn validate_length(&self) -> Result<()> {
        if self.data.len() > 2097151 {
            // Standard Minecraft max packet length
            return Err(PacketError::InvalidLength {
                length: self.data.len(),
                max: 2097151,
            });
        }
        Ok(())
    }

    fn validate_encryption(&self) -> Result<()> {
        if let EncryptionState::Enabled {
            encrypted_data: false,
        } = self.encryption
        {
            if !self.data.is_empty() {
                return Err(PacketError::Encryption(
                    "Data not encrypted when encryption is enabled".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn validate_compression(&self) -> Result<()> {
        if let CompressionState::Enabled { threshold } = self.compression {
            if threshold < 0 {
                return Err(PacketError::Compression(
                    "Invalid compression threshold".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl PacketCodec for MockPacket {
    fn encode<T: ProtocolWrite>(&mut self, value: &T) -> Result<()> {
        let mut cursor = Cursor::new(Vec::new());
        value.write_to(&mut cursor).map_err(PacketError::Io)?;
        self.data.extend_from_slice(&cursor.into_inner());
        Ok(())
    }

    fn decode<T: ProtocolRead>(&self) -> Result<T> {
        let mut cursor = Cursor::new(&self.data[..]);
        let (value, _) = T::read_from(&mut cursor).map_err(PacketError::Io)?;
        Ok(value)
    }
}

impl PacketSerialization for MockPacket {
    fn into_raw_bytes(self) -> Result<BytesMut> {
        let mut output = BytesMut::new();

        // Encode the packet ID as a VarInt
        let mut id_bytes = BytesMut::new();
        let id = VarInt(self.id);
        id.write_to_bytes(&mut id_bytes).map_err(PacketError::Io)?;

        // Calculate total length
        let total_length = VarInt((id_bytes.len() + self.data.len()) as i32);

        // Write total length
        total_length
            .write_to_bytes(&mut output)
            .map_err(PacketError::Io)?;

        // Append ID and data
        output.extend_from_slice(&id_bytes);
        output.extend_from_slice(&self.data);

        Ok(output)
    }

    fn from_raw_bytes(bytes: BytesMut) -> Result<Self> {
        let mut cursor = Cursor::new(&bytes[..]);

        // Read packet length
        let (VarInt(length), length_size) = VarInt::read_from(&mut cursor)
            .map_err(|_| PacketError::InvalidFormat("Invalid packet length VarInt".to_string()))?;

        if length <= 0 || length as usize > 2097151 {
            // MAX_PACKET_LENGTH
            return Err(PacketError::InvalidLength {
                length: length as usize,
                max: 2097151,
            });
        }

        // Skip to the packet ID (move cursor past the length bytes)
        let pos = length_size;
        let remaining = &bytes[pos..];
        let mut id_cursor = Cursor::new(remaining);

        // Read packet ID
        let (VarInt(id), id_size) = VarInt::read_from(&mut id_cursor)
            .map_err(|_| PacketError::InvalidFormat("Invalid packet ID VarInt".to_string()))?;

        // Get the data portion (excluding the ID)
        let data_start = pos + id_size;
        let data = BytesMut::from(&bytes[data_start..]);

        Ok(MockPacket {
            id,
            data,
            protocol_version: Version::V1_20_2,
            compression: CompressionState::Disabled,
            encryption: EncryptionState::Disabled,
        })
    }
}

/// A factory for creating mock packets
pub struct MockPacketFactory;

impl PacketFactory for MockPacketFactory {
    type Packet = MockPacket;

    fn create_packet(&self, id: i32) -> Self::Packet {
        MockPacket::new(id)
    }

    fn create_from_bytes(&self, bytes: BytesMut) -> Result<Self::Packet> {
        MockPacket::from_raw_bytes(bytes)
    }
}
