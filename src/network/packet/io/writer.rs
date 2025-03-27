use aes::cipher::BlockEncryptMut;
use async_trait::async_trait;
use bytes::BytesMut;
use libdeflater::CompressionLvl;
use libdeflater::Compressor;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::network::proxy_protocol::ProtocolResult;
use crate::protocol::types::{VarInt, WriteToBytes};
use crate::security::encryption::Aes128Cfb8Enc;
use crate::security::encryption::Cfb8Closure;

use super::super::{
    base::{CompressionState, Packet},
    error::{PacketError, PacketResult},
};

use super::RawPacketIO;

/// Handles packet writing with compression and encryption support
pub struct PacketWriter<W> {
    writer: W,
    encryption: Option<Aes128Cfb8Enc>,
    compression: CompressionState,
}

impl<W: AsyncWrite + Unpin + Send> PacketWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            encryption: None,
            compression: CompressionState::Disabled,
        }
    }

    pub async fn flush(&mut self) {
        self.writer.flush().await.unwrap();
    }

    pub fn get_ref(&self) -> &W {
        &self.writer
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    pub fn enable_encryption(&mut self, cipher: Aes128Cfb8Enc) {
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

    pub fn is_encryption_enabled(&self) -> bool {
        self.encryption.is_some()
    }

    pub fn is_compression_enabled(&self) -> bool {
        self.compression != CompressionState::Disabled
    }

    pub fn get_compress_threshold(&self) -> i32 {
        match self.compression {
            CompressionState::Enabled { threshold } => threshold,
            _ => 0,
        }
    }

    pub async fn write_packet(&mut self, packet: &Packet) -> ProtocolResult<()> {
        let mut packet_data = BytesMut::new();

        // Write packet ID and data
        VarInt(packet.id).write_to_bytes(&mut packet_data)?;
        packet_data.extend_from_slice(&packet.data);

        // Handle compression if enabled
        let final_data = if self.is_compression_enabled() {
            let threshold = self.get_compress_threshold();
            if packet_data.len() >= threshold as usize {
                let mut compressor = Compressor::new(CompressionLvl::default());
                let max_sz = compressor.zlib_compress_bound(packet_data.len());
                let mut compressed_data = vec![0; max_sz];

                let actual_sz = compressor
                    .zlib_compress(&packet_data, &mut compressed_data)
                    .unwrap();
                compressed_data.resize(actual_sz, 0);

                let mut compressed_packet = BytesMut::new();
                VarInt(packet_data.len() as i32).write_to_bytes(&mut compressed_packet)?;
                compressed_packet.extend_from_slice(&compressed_data);
                compressed_packet
            } else {
                let mut uncompressed = BytesMut::new();
                VarInt(0).write_to_bytes(&mut uncompressed)?;
                uncompressed.extend_from_slice(&packet_data);
                uncompressed
            }
        } else {
            packet_data
        };

        // Write total length and data
        let mut output = BytesMut::new();
        VarInt(final_data.len() as i32).write_to_bytes(&mut output)?;
        output.extend_from_slice(&final_data);

        // Handle encryption if enabled
        let mut encrypted_data = output.clone();
        if let Some(cipher) = &mut self.encryption {
            cipher.encrypt_with_backend_mut(Cfb8Closure {
                data: &mut encrypted_data,
            });
        }

        // Write final data and flush
        self.writer.write_all(&encrypted_data).await?;
        self.writer.flush().await?;

        Ok(())
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> PacketResult<()> {
        self.writer.write_all(data).await.map_err(PacketError::Io)?;
        self.writer.flush().await.map_err(PacketError::Io)?;
        Ok(())
    }

    pub async fn close(&mut self) -> PacketResult<()> {
        self.writer.shutdown().await.map_err(PacketError::Io)?;
        Ok(())
    }
}

#[async_trait]
impl<W> RawPacketIO for PacketWriter<W>
where
    W: AsyncWrite + Unpin + Send,
{
    async fn read_raw(&mut self) -> PacketResult<Option<BytesMut>> {
        Err(PacketError::invalid_format("Writers cannot read"))
    }

    async fn write_raw(&mut self, data: &[u8]) -> PacketResult<()> {
        // Directly write data without modification
        self.writer.write_all(data).await.map_err(PacketError::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use tokio::io::BufWriter;

    #[tokio::test]
    async fn test_write_simple_packet() {
        let buffer = Vec::new();
        let mut writer = PacketWriter::new(BufWriter::new(buffer));

        let mut data = BytesMut::new();
        data.put_slice(&[1, 2, 3]);

        let packet = Packet {
            id: 0,
            data,
            compression: CompressionState::Disabled,
            encryption: crate::network::packet::EncryptionState::Disabled,
            protocol_version: crate::protocol::version::Version::V1_20_2,
        };

        writer.write_packet(&packet).await.unwrap();

        let written = writer.writer.into_inner();
        assert_eq!(written[0], 4); // Total length (VarInt)
        assert_eq!(written[1], 0); // Packet ID (VarInt)
        assert_eq!(&written[2..], &[1, 2, 3]); // Data
    }

    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_write_compressed_packet() {
        let buffer = Vec::new();
        let mut writer = PacketWriter::new(BufWriter::new(buffer));
        writer.enable_compression(256);
        //TODO: Implement test with actual compression
    }

    #[tokio::test]
    #[ignore = "TODO"]
    async fn test_write_encrypted_packet() {
        let buffer = Vec::new();
        let mut _writer = PacketWriter::new(BufWriter::new(buffer));
        //TODO: Implement test with actual encryption
    }
}
