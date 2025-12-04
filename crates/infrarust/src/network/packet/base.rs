use bytes::{Buf, BytesMut};
use std::io::Cursor;

use infrarust_protocol::{
    packet::{
        CompressionControl, CompressionState, EncryptionControl, EncryptionState, PacketCodec,
        PacketDataAccess, PacketError, PacketSerialization, PacketValidation, Result,
    },
    types::{ProtocolRead, ProtocolWrite, VarInt, WriteToBytes},
    version::Version,
};

use super::PacketBuilder;

// Constants
pub const MAX_PACKET_LENGTH: usize = 2097151; // 2^21 - 1 (3-byte VarInt max)
pub const MAX_PACKET_DATA_LENGTH: usize = 0x200000; // 2MB
pub const MAX_UNCOMPRESSED_LENGTH: usize = 8388608; // 2^23

#[derive(Clone)]
pub struct Packet {
    pub id: i32,
    pub data: BytesMut,
    pub compression: CompressionState,
    pub encryption: EncryptionState,
    pub protocol_version: Version,
}

impl std::fmt::Debug for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Packet")
            .field("id", &format!("0x{:02x}", self.id))
            .field("data_len", &self.data.len())
            .field("compression", &self.compression)
            .field("encryption", &self.encryption)
            .field("protocol_version", &self.protocol_version)
            .finish()
    }
}

impl Packet {
    pub fn new(id: i32) -> Self {
        Self {
            id,
            data: BytesMut::new(),
            compression: CompressionState::Disabled,
            encryption: EncryptionState::Disabled,
            protocol_version: Version::V1_20_2,
        }
    }

    pub fn with_capacity(id: i32, capacity: usize) -> Self {
        Self {
            id,
            data: BytesMut::with_capacity(capacity),
            compression: CompressionState::Disabled,
            encryption: EncryptionState::Disabled,
            protocol_version: Version::V1_20_2,
        }
    }
}

impl PacketDataAccess for Packet {
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

impl CompressionControl for Packet {
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

impl EncryptionControl for Packet {
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

impl PacketValidation for Packet {
    fn validate_length(&self) -> Result<()> {
        if let EncryptionState::Enabled { encrypted_data: _ } = self.encryption {
            return Ok(());
        }

        if self.data.len() > MAX_PACKET_LENGTH {
            return Err(PacketError::InvalidLength {
                length: self.data.len(),
                max: MAX_PACKET_LENGTH,
            });
        }
        Ok(())
    }

    fn validate_encryption(&self) -> Result<()> {
        match self.encryption {
            EncryptionState::Enabled {
                encrypted_data: false,
            } => {
                if self.data.is_empty() {
                    Ok(())
                } else {
                    Err(PacketError::Encryption(
                        "Non-encrypted data when encryption is enabled".to_string(),
                    ))
                }
            }
            _ => Ok(()),
        }
    }

    fn validate_compression(&self) -> Result<()> {
        if let CompressionState::Enabled { threshold } = self.compression
            && threshold < 0
        {
            return Err(PacketError::Compression(
                "Invalid compression threshold".to_string(),
            ));
        }
        Ok(())
    }
}

impl PacketCodec for Packet {
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

impl PacketSerialization for Packet {
    fn into_raw_bytes(self) -> Result<BytesMut> {
        let mut output = BytesMut::new();

        let mut packet_content = BytesMut::new();
        VarInt(self.id).write_to_bytes(&mut packet_content)?;
        packet_content.extend_from_slice(&self.data);

        let total_length = VarInt(packet_content.len() as i32);
        total_length.write_to_bytes(&mut output)?;

        output.extend_from_slice(&packet_content);

        Ok(output)
    }

    fn from_raw_bytes(mut bytes: BytesMut) -> Result<Self> {
        // Read and validate packet length
        let (VarInt(length), length_size) = VarInt::read_from(&mut Cursor::new(&bytes[..]))
            .map_err(|_| PacketError::InvalidFormat("Invalid packet length VarInt".to_string()))?;

        if length <= 0 || length as usize > MAX_PACKET_LENGTH {
            return Err(PacketError::InvalidLength {
                length: length as usize,
                max: MAX_PACKET_LENGTH,
            });
        }

        bytes.advance(length_size);

        let (VarInt(id), id_size) = VarInt::read_from(&mut Cursor::new(&bytes[..]))
            .map_err(|_| PacketError::InvalidFormat("Invalid packet ID VarInt".to_string()))?;

        bytes.advance(id_size);

        if bytes.len() > MAX_PACKET_DATA_LENGTH {
            return Err(PacketError::InvalidLength {
                length: bytes.len(),
                max: MAX_PACKET_DATA_LENGTH,
            });
        }

        let packet = PacketBuilder::new().id(id).data(bytes).build()?;
        packet.validate_length()?;

        Ok(packet)
    }
}
