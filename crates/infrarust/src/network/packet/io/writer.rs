use std::cell::RefCell;

use aes::cipher::BlockEncryptMut;
use async_trait::async_trait;
use bytes::BytesMut;
use infrarust_protocol::packet::CompressionState;
use infrarust_protocol::types::VarInt;
use infrarust_protocol::types::WriteToBytes;
use libdeflater::CompressionLvl;
use libdeflater::Compressor;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::network::proxy_protocol::ProtocolResult;
use crate::security::encryption::Aes128Cfb8Enc;
use crate::security::encryption::Cfb8Closure;

use super::super::{
    base::Packet,
    error::{PacketError, PacketResult},
};

use super::RawPacketIO;
use super::buffer_pool::{get_buffer_with_capacity, return_buffer};

/// Handles packet writing with compression and encryption support
// Thread-local compressor pool to avoid per-packet allocation while maintaining Send+Sync
thread_local! {
    static COMPRESSOR_POOL: RefCell<Vec<Compressor>> = const { RefCell::new(Vec::new()) };
}
fn get_compressor() -> Compressor {
    COMPRESSOR_POOL.with(|pool| {
        pool.borrow_mut()
            .pop()
            .unwrap_or_else(|| Compressor::new(CompressionLvl::default()))
    })
}

fn return_compressor(compressor: Compressor) {
    COMPRESSOR_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < 4 {
            pool.push(compressor);
        }
    });
}
#[derive(Clone, Debug)]
pub struct PacketWriter<W> {
    writer: W,
    encryption: Option<Aes128Cfb8Enc>,
    compression: CompressionState,

    packet_buffer: BytesMut,
    output_buffer: BytesMut,
    compressed_buffer: BytesMut,
}

impl<W: AsyncWrite + Unpin + Send> PacketWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            encryption: None,
            compression: CompressionState::Disabled,

            packet_buffer: BytesMut::with_capacity(8192),
            output_buffer: BytesMut::with_capacity(8192),
            compressed_buffer: BytesMut::with_capacity(8192),
        }
    }

    pub async fn flush(&mut self) -> PacketResult<()> {
        self.writer.flush().await?;
        Ok(())
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
        self.packet_buffer.clear();
        self.output_buffer.clear();
        self.compressed_buffer.clear();

        // Write packet ID and data
        VarInt(packet.id).write_to_bytes(&mut self.packet_buffer)?;
        self.packet_buffer.extend_from_slice(&packet.data);

        // Handle compression if enabled
        if self.is_compression_enabled() {
            let threshold = self.get_compress_threshold();
            if self.packet_buffer.len() >= threshold as usize {
                let mut compressor = get_compressor();
                let max_sz = compressor.zlib_compress_bound(self.packet_buffer.len());
                let mut compressed_data = get_buffer_with_capacity(max_sz);
                // SAFETY: We're about to overwrite this memory with compressed data.
                // Using resize with 0 would unnecessarily zero memory that gets overwritten.
                unsafe {
                    compressed_data.set_len(max_sz);
                }

                let compress_result = compressor
                    .zlib_compress(&self.packet_buffer, &mut compressed_data)
                    .map_err(|e| PacketError::Compression(format!("Compression failed: {:?}", e)));
                return_compressor(compressor);
                let actual_sz = compress_result?;

                VarInt(self.packet_buffer.len() as i32)
                    .write_to_bytes(&mut self.compressed_buffer)?;
                self.compressed_buffer.extend_from_slice(&compressed_data[..actual_sz]);

                return_buffer(compressed_data);

                VarInt(self.compressed_buffer.len() as i32).write_to_bytes(&mut self.output_buffer)?;
                self.output_buffer.extend_from_slice(&self.compressed_buffer);
            } else {
                // Uncompressed: [total_length][0][packet_data]
                VarInt(0).write_to_bytes(&mut self.compressed_buffer)?;
                self.compressed_buffer.extend_from_slice(&self.packet_buffer);

                VarInt(self.compressed_buffer.len() as i32).write_to_bytes(&mut self.output_buffer)?;
                self.output_buffer.extend_from_slice(&self.compressed_buffer);
            }
        } else {
            // No compression: [total_length][packet_data]
            VarInt(self.packet_buffer.len() as i32).write_to_bytes(&mut self.output_buffer)?;
            self.output_buffer.extend_from_slice(&self.packet_buffer);
        }

        // Handle encryption if enabled (in-place)
        if let Some(cipher) = &mut self.encryption {
            cipher.encrypt_with_backend_mut(Cfb8Closure {
                data: &mut self.output_buffer,
            });
        }

        // Write final data and flush
        self.writer.write_all(&self.output_buffer).await?;
        self.writer.flush().await?;

        Ok(())
    }

    pub async fn write_raw(&mut self, data: &[u8]) -> PacketResult<()> {
        self.writer.write_all(data).await.map_err(PacketError::Io)?;
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
        Err(PacketError::InvalidFormat(
            "Writers cannot read".to_string(),
        ))
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
    use tokio::io::BufWriter;

    // #[tokio::test]
    // async fn test_write_simple_packet() {
    //     let buffer = Vec::new();
    //     let mut writer = PacketWriter::new(BufWriter::new(buffer));

    //     let mut data = BytesMut::new();
    //     data.put_slice(&[1, 2, 3]);

    //     let packet = Packet {
    //         id: 0,
    //         data,
    //         compression: CompressionState::Disabled,
    //         encryption: crate::network::packet::EncryptionState::Disabled,
    //         protocol_version: crate::protocol::version::Version::V1_20_2,
    //     };

    //     writer.write_packet(&packet).await.unwrap();

    //     let written = into_inner();
    //     assert_eq!(written[0], 4); // Total length (VarInt)
    //     assert_eq!(written[1], 0); // Packet ID (VarInt)
    //     assert_eq!(&written[2..], &[1, 2, 3]); // Data
    // }

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
