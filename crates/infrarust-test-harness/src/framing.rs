use std::net::SocketAddr;

use infrarust_protocol::crypto::{DecryptCipher, EncryptCipher};
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::error::HarnessResult;

const READ_CHUNK: usize = 16 * 1024;

pub struct FramedConn {
    reader: FrameReader,
    writer: FrameWriter,
}

impl FramedConn {
    pub async fn connect(addr: SocketAddr) -> HarnessResult<Self> {
        Self::new(TcpStream::connect(addr).await?)
    }

    pub fn new(stream: TcpStream) -> HarnessResult<Self> {
        stream.set_nodelay(true)?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: FrameReader {
                half: read_half,
                decoder: PacketDecoder::new(),
                decrypt: None,
            },
            writer: FrameWriter {
                half: write_half,
                encoder: PacketEncoder::new(),
                encrypt: None,
            },
        })
    }

    pub async fn read_frame(&mut self) -> HarnessResult<Option<PacketFrame>> {
        self.reader.read_frame().await
    }

    pub async fn write_frame(&mut self, frame: &PacketFrame) -> HarnessResult<()> {
        self.writer.write_frame(frame).await
    }

    pub fn set_compression(&mut self, threshold: Option<i32>) {
        self.reader.set_compression(threshold);
        self.writer.set_compression(threshold);
    }

    pub const fn compression(&self) -> Option<i32> {
        self.writer.compression()
    }

    pub fn enable_encryption(&mut self, key: &[u8; 16]) {
        self.reader.enable_decryption(key);
        self.writer.enable_encryption(key);
    }

    pub fn peer_addr(&self) -> HarnessResult<SocketAddr> {
        Ok(self.reader.half.peer_addr()?)
    }

    pub fn into_split(self) -> (FrameReader, FrameWriter) {
        (self.reader, self.writer)
    }
}

pub struct FrameReader {
    half: OwnedReadHalf,
    decoder: PacketDecoder,
    decrypt: Option<DecryptCipher>,
}

impl FrameReader {
    pub async fn read_frame(&mut self) -> HarnessResult<Option<PacketFrame>> {
        loop {
            if let Some(frame) = self.decoder.try_next_frame()? {
                return Ok(Some(frame));
            }
            let buf = self.decoder.read_buf_mut();
            buf.reserve(READ_CHUNK);
            let old_len = buf.len();
            if self.half.read_buf(buf).await? == 0 {
                return Ok(None);
            }
            if let Some(cipher) = &mut self.decrypt {
                cipher.decrypt(&mut self.decoder.read_buf_mut()[old_len..]);
            }
        }
    }

    pub fn set_compression(&mut self, threshold: Option<i32>) {
        self.decoder.set_compression(threshold.unwrap_or(-1));
    }

    pub fn enable_decryption(&mut self, key: &[u8; 16]) {
        self.decrypt = Some(DecryptCipher::new(key));
    }
}

pub struct FrameWriter {
    half: OwnedWriteHalf,
    encoder: PacketEncoder,
    encrypt: Option<EncryptCipher>,
}

impl FrameWriter {
    pub async fn write_frame(&mut self, frame: &PacketFrame) -> HarnessResult<()> {
        self.encoder.append_frame(frame)?;
        let mut data = self.encoder.take();
        if let Some(cipher) = &mut self.encrypt {
            cipher.encrypt(&mut data);
        }
        self.half.write_all(&data).await?;
        Ok(())
    }

    pub fn set_compression(&mut self, threshold: Option<i32>) {
        self.encoder.set_compression(threshold.unwrap_or(-1));
    }

    pub const fn compression(&self) -> Option<i32> {
        self.encoder.compression_threshold()
    }

    pub fn enable_encryption(&mut self, key: &[u8; 16]) {
        self.encrypt = Some(EncryptCipher::new(key));
    }

    pub async fn shutdown(&mut self) -> HarnessResult<()> {
        self.half.shutdown().await?;
        Ok(())
    }
}
