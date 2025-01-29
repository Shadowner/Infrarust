use std::io::Cursor;

use aes::cipher::BlockDecryptMut;
use async_trait::async_trait;
use bytes::BytesMut;
use libdeflater::Decompressor;
use tokio::io::{AsyncRead, AsyncReadExt};

use super::super::{
    base::{CompressionState, EncryptionState, Packet},
    error::{PacketError, PacketResult},
};
use super::RawPacketIO;
use crate::version::Version;
use crate::{
    network::packet::MAX_UNCOMPRESSED_LENGTH,
    security::encryption::{Aes128Cfb8Dec, Cfb8Closure},
};
use crate::{protocol::types::VarInt, ProtocolRead};

/// Handles packet reading with support for compression and encryption
pub struct PacketReader<R> {
    pub reader: R,
    encryption: Option<Aes128Cfb8Dec>,
    compression: CompressionState,
    protocol_version: Version,
}

impl<R: AsyncRead + Unpin> PacketReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            encryption: None,
            compression: CompressionState::Disabled,
            protocol_version: Version::new(0),
        }
    }

    pub fn is_encryption_enabled(&self) -> bool {
        self.encryption.is_some()
    }

    pub fn enable_encryption(&mut self, cipher: Aes128Cfb8Dec) {
        self.encryption = Some(cipher);
    }

    pub fn disable_encryption(&mut self) {
        self.encryption = None;
    }

    pub fn enable_compression(&mut self, threshold: i32) {
        self.compression = CompressionState::Enabled { threshold };
    }

    pub fn disable_compression(&mut self) {
        self.compression = CompressionState::Disabled;
    }

    pub fn is_compressing(&self) -> bool {
        matches!(self.compression, CompressionState::Enabled { .. })
    }

    pub fn set_protocol_version(&mut self, version: Version) {
        self.protocol_version = version;
    }

    pub async fn read_packet(&mut self) -> PacketResult<Packet> {
        // Read total packet length (may be encrypted)
        let packet_length = {
            let mut length_bytes = BytesMut::new();
            loop {
                let mut byte = [0u8; 1];
                self.reader.read_exact(&mut byte).await?;

                if let Some(cipher) = &mut self.encryption {
                    cipher.decrypt_with_backend_mut(Cfb8Closure { data: &mut byte });
                }

                length_bytes.extend_from_slice(&byte);
                if byte[0] & 0x80 == 0 {
                    break;
                }
                if length_bytes.len() >= 3 {
                    return Err(PacketError::VarIntDecoding("VarInt too long".to_string()));
                }
            }

            let mut cursor = Cursor::new(&length_bytes);
            let (VarInt(length), _) = VarInt::read_from(&mut cursor)?;
            length
        };

        // debug!("Reading packet with total length: {}", packet_length);

        // Read packet data (may be encrypted)
        let mut encrypted_data = vec![0u8; packet_length as usize];
        self.reader.read_exact(&mut encrypted_data).await?;

        // Handle decryption if enabled
        if let Some(cipher) = &mut self.encryption {
            cipher.decrypt_with_backend_mut(Cfb8Closure {
                data: &mut encrypted_data,
            });
        }

        // Handle decompression if enabled
        let packet_data = if let CompressionState::Enabled { threshold: _ } = self.compression {
            let mut cursor = Cursor::new(&encrypted_data);
            let (VarInt(data_length), bytes_read) = VarInt::read_from(&mut cursor)?;
            // debug!("Data length (uncompressed): {}", data_length);

            if data_length == 0 {
                BytesMut::from(&encrypted_data[bytes_read..])
            } else {
                if data_length > MAX_UNCOMPRESSED_LENGTH as i32 {
                    return Err(PacketError::InvalidLength {
                        length: data_length as usize,
                        max: MAX_UNCOMPRESSED_LENGTH,
                    });
                }

                let mut decompressor = Decompressor::new();
                let mut outbuf = vec![0; data_length as usize];

                decompressor
                    .zlib_decompress(&encrypted_data[bytes_read..], &mut outbuf)
                    .unwrap();

                if outbuf.len() != data_length as usize {
                    return Err(PacketError::compression("Decompressed length mismatch"));
                }

                BytesMut::from(&outbuf[..])
            }
        } else {
            BytesMut::from(&encrypted_data[..])
        };

        // Read packet ID and create final packet
        let mut cursor = Cursor::new(&packet_data);
        let (VarInt(id), id_size) = VarInt::read_from(&mut cursor)?;

        Ok(Packet {
            id,
            data: BytesMut::from(&packet_data[id_size..]),
            compression: self.compression,
            encryption: if self.encryption.is_some() {
                EncryptionState::Enabled {
                    encrypted_data: true,
                }
            } else {
                EncryptionState::Disabled
            },
            protocol_version: self.protocol_version,
        })
    }

    pub fn get_ref(&self) -> &R {
        &self.reader
    }

    pub fn get_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

#[async_trait]
impl<R> RawPacketIO for PacketReader<R>
where
    R: AsyncRead + Unpin + Send,
{
    async fn read_raw(&mut self) -> PacketResult<Option<BytesMut>> {
        let mut buffer = BytesMut::with_capacity(8192);
        match self.reader.read_buf(&mut buffer).await {
            Ok(0) => Ok(None), // EOF
            Ok(_) => Ok(Some(buffer)),
            Err(e) => Err(PacketError::Io(e)),
        }
    }

    async fn write_raw(&mut self, _data: &[u8]) -> PacketResult<()> {
        Err(PacketError::invalid_format("Readers cannot write"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn test_read_simple_packet() {
        // Create simple packet: [total length][id][data]
        let mut data = Vec::new();
        data.extend_from_slice(&[3]); // Length
        data.extend_from_slice(&[0]); // ID
        data.extend_from_slice(&[1, 2]); // Data

        let cursor = Cursor::new(data);
        let mut reader = PacketReader::new(BufReader::new(cursor));

        let packet = reader.read_packet().await.unwrap();
        assert_eq!(packet.id, 0);
        assert_eq!(&packet.data[..], &[1, 2]);
    }

    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_read_compressed_packet() {
        let mut reader = PacketReader::new(BufReader::new(Cursor::new(Vec::new())));
        reader.enable_compression(256);
        //TODO: Test à implémenter avec des données compressées réelles
    }

    #[tokio::test]
    async fn test_invalid_packet_length() {
        let cursor = Cursor::new(vec![0]); // Longueur invalide (0)
        let mut reader = PacketReader::new(BufReader::new(cursor));

        let result = reader.read_packet().await;
        assert!(result.is_err());
    }
}
