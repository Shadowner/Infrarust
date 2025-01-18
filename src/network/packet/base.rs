use bytes::{Buf, BytesMut};
use std::io::Cursor;

use crate::{
    network::packet::{PacketBuilder, MAX_PACKET_DATA_LENGTH, MAX_PACKET_LENGTH},
    protocol::{
        types::{ProtocolRead, ProtocolWrite, VarInt, WriteToBytes},
        version::Version,
    },
};

use super::error::{PacketError, PacketResult};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompressionState {
    Disabled,
    Enabled { threshold: i32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum EncryptionState {
    Disabled,
    Enabled { encrypted_data: bool },
}

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

pub trait PacketValidation {
    fn validate_length(&self) -> PacketResult<()>;
    fn validate_encryption(&self) -> PacketResult<()>;
    fn validate_compression(&self) -> PacketResult<()>;
}

pub trait PacketCodec {
    fn encode<T: ProtocolWrite>(&mut self, value: &T) -> PacketResult<()>;
    fn decode<T: ProtocolRead>(&self) -> PacketResult<T>;
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

    pub fn set_protocol_version(&mut self, version: Version) {
        self.protocol_version = version;
    }

    pub fn enable_compression(&mut self, threshold: i32) {
        self.compression = CompressionState::Enabled { threshold };
    }

    pub fn disable_compression(&mut self) {
        self.compression = CompressionState::Disabled;
    }

    pub fn enable_encryption(&mut self) {
        self.encryption = EncryptionState::Enabled {
            encrypted_data: false,
        };
    }

    pub fn disable_encryption(&mut self) {
        self.encryption = EncryptionState::Disabled;
    }

    pub fn mark_as_encrypted(&mut self) {
        if let EncryptionState::Enabled {
            ref mut encrypted_data,
        } = self.encryption
        {
            *encrypted_data = true;
        }
    }

    pub fn from_bytes(mut bytes: BytesMut) -> PacketResult<Self> {
        use std::io::Cursor;

        // Read and validate packet length
        let (VarInt(length), length_size) = VarInt::read_from(&mut Cursor::new(&bytes[..]))
            .map_err(|_| PacketError::invalid_format("Invalid packet length VarInt"))?;

        if length <= 0 || length as usize > MAX_PACKET_LENGTH {
            return Err(PacketError::InvalidLength {
                length: length as usize,
                max: MAX_PACKET_LENGTH,
            });
        }

        bytes.advance(length_size);

        let (VarInt(id), id_size) = VarInt::read_from(&mut Cursor::new(&bytes[..]))
            .map_err(|_| PacketError::invalid_format("Invalid packet ID VarInt"))?;

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

    pub fn into_raw(self) -> PacketResult<BytesMut> {
        let mut output = BytesMut::new();

        let mut packet_content = BytesMut::new();
        VarInt(self.id).write_to_bytes(&mut packet_content)?;
        packet_content.extend_from_slice(&self.data);

        let total_length = VarInt(packet_content.len() as i32);
        total_length.write_to_bytes(&mut output)?;

        output.extend_from_slice(&packet_content);

        Ok(output)
    }
}

impl PacketValidation for Packet {
    fn validate_length(&self) -> PacketResult<()> {
        if let EncryptionState::Enabled { encrypted_data: _ } = self.encryption {
            return Ok(());
        }
        const MAX_PACKET_LENGTH: usize = 2097151;

        if self.data.len() > MAX_PACKET_LENGTH {
            return Err(PacketError::InvalidLength {
                length: self.data.len(),
                max: MAX_PACKET_LENGTH,
            });
        }
        Ok(())
    }

    fn validate_encryption(&self) -> PacketResult<()> {
        match self.encryption {
            EncryptionState::Enabled {
                encrypted_data: false,
            } => {
                if self.data.is_empty() {
                    Ok(())
                } else {
                    Err(PacketError::encryption(
                        "Données non chiffrées alors que le chiffrement est activé",
                    ))
                }
            }
            _ => Ok(()),
        }
    }

    fn validate_compression(&self) -> PacketResult<()> {
        if let CompressionState::Enabled { threshold } = self.compression {
            if threshold < 0 {
                return Err(PacketError::compression("Seuil de compression invalide"));
            }
        }
        Ok(())
    }
}

impl PacketCodec for Packet {
    fn encode<T: ProtocolWrite>(&mut self, value: &T) -> PacketResult<()> {
        let mut cursor = Cursor::new(Vec::new());
        value.write_to(&mut cursor).map_err(PacketError::Io)?;
        self.data.extend_from_slice(&cursor.into_inner());
        Ok(())
    }

    fn decode<T: ProtocolRead>(&self) -> PacketResult<T> {
        let mut cursor = Cursor::new(&self.data[..]);
        let (value, _) = T::read_from(&mut cursor).map_err(PacketError::Io)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::ProtocolString;

    #[test]
    fn test_packet_creation() {
        let packet = Packet::new(0x00);
        assert_eq!(packet.id, 0x00);
        assert_eq!(packet.data.len(), 0);
        assert_eq!(packet.compression, CompressionState::Disabled);
        assert_eq!(packet.encryption, EncryptionState::Disabled);
    }

    #[test]
    fn test_packet_compression_state() {
        let mut packet = Packet::new(0x00);
        packet.enable_compression(256);
        assert_eq!(
            packet.compression,
            CompressionState::Enabled { threshold: 256 }
        );
        packet.disable_compression();
        assert_eq!(packet.compression, CompressionState::Disabled);
    }

    #[test]
    fn test_packet_encryption_state() {
        let mut packet = Packet::new(0x00);
        packet.enable_encryption();
        assert_eq!(
            packet.encryption,
            EncryptionState::Enabled {
                encrypted_data: false
            }
        );
        packet.mark_as_encrypted();
        assert_eq!(
            packet.encryption,
            EncryptionState::Enabled {
                encrypted_data: true
            }
        );
    }

    #[test]
    fn test_packet_validation() {
        let mut packet = Packet::with_capacity(0x00, 10);
        packet.data.extend_from_slice(&[0; 2097151]);
        assert!(packet.validate_length().is_ok());

        packet.data.extend_from_slice(&[0; 1]);
        assert!(packet.validate_length().is_err());
    }

    #[test]
    fn test_packet_codec() {
        let mut packet = Packet::new(0x00);

        let test_string = ProtocolString("Hello".to_string());
        assert!(packet.encode(&test_string).is_ok());

        let decoded: ProtocolString = packet.decode().unwrap();
        assert_eq!(decoded.0, "Hello");
    }
}
