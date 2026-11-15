use std::io::ErrorKind;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use infrarust_protocol::Packet;
use infrarust_protocol::crypto::{DecryptCipher, EncryptCipher};
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::pipeline::preauth_frame::refuse_oversized;
use crate::session::bounded_write::WriteGuard;
use crate::util::packets::encode_for_state;

const READ_CHUNK: usize = 16 * 1024;
const MAX_HELD_BYTES: usize = 1024 * 1024;

pub struct FramedStream {
    stream: TcpStream,
    writes: WriteGuard,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    encrypt_cipher: Option<EncryptCipher>,
    decrypt_cipher: Option<DecryptCipher>,
    protocol_version: ProtocolVersion,
    state: ConnectionState,
}

impl FramedStream {
    pub(crate) fn new(stream: TcpStream, protocol_version: ProtocolVersion) -> Self {
        Self {
            stream,
            writes: WriteGuard::new(),
            decoder: PacketDecoder::new(),
            encoder: PacketEncoder::new(),
            encrypt_cipher: None,
            decrypt_cipher: None,
            protocol_version,
            state: ConnectionState::Login,
        }
    }

    pub(crate) fn queue_received(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.decoder.queue_bytes(bytes);
        }
    }

    pub(crate) const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    pub fn set_cancel(&mut self, cancel: CancellationToken) {
        self.writes.set_cancel(cancel);
    }

    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    pub const fn state(&self) -> ConnectionState {
        self.state
    }

    pub const fn set_state(&mut self, state: ConnectionState) {
        self.state = state;
    }

    pub async fn read_frame(&mut self) -> Result<Option<PacketFrame>, CoreError> {
        loop {
            if let Some(frame) = self.decoder.try_next_frame()? {
                return Ok(Some(frame));
            }

            if self.fill_decoder().await? == 0 {
                return Ok(None);
            }
        }
    }

    pub(crate) async fn read_preauth_frame(
        &mut self,
        limit: usize,
    ) -> Result<Option<PacketFrame>, CoreError> {
        loop {
            refuse_oversized(self.decoder.read_buf_mut(), limit)?;
            if let Some(frame) = self.decoder.try_next_frame()? {
                return Ok(Some(frame));
            }

            if self.fill_decoder().await? == 0 {
                return Ok(None);
            }
        }
    }

    async fn fill_decoder(&mut self) -> Result<usize, CoreError> {
        loop {
            self.stream.readable().await?;
            let buf = self.decoder.read_buf_mut();
            buf.reserve(READ_CHUNK);
            let old_len = buf.len();
            match self.stream.try_read_buf(buf) {
                Ok(n) => {
                    if let Some(cipher) = &mut self.decrypt_cipher {
                        cipher.decrypt(&mut self.decoder.read_buf_mut()[old_len..]);
                    }
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub(crate) async fn closed(&mut self) -> CoreError {
        while self.decoder.read_buf_mut().len() < MAX_HELD_BYTES {
            match self.fill_decoder().await {
                Ok(0) => return CoreError::ConnectionClosed,
                Ok(_) => {}
                Err(e) => return e,
            }
        }
        std::future::pending().await
    }

    pub fn try_next_frame(&mut self) -> Result<Option<PacketFrame>, CoreError> {
        Ok(self.decoder.try_next_frame()?)
    }

    pub fn queue_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        Ok(self.encoder.append_frame(frame)?)
    }

    pub(crate) fn queue_raw(&mut self, id: i32, payload: &[u8]) -> Result<(), CoreError> {
        Ok(self.encoder.append_raw(id, payload)?)
    }

    pub async fn flush(&mut self) -> Result<(), CoreError> {
        let mut data = self.encoder.take();
        if data.is_empty() {
            return self.writes.check();
        }
        if let Some(cipher) = &mut self.encrypt_cipher {
            cipher.encrypt(&mut data);
        }
        self.writes.write_all(&mut self.stream, &data).await
    }

    pub async fn write_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        self.queue_frame(frame)?;
        self.flush().await
    }

    pub async fn send_packet<P: Packet>(
        &mut self,
        packet: &P,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let frame = encode_for_state(packet, self.state, self.protocol_version, registry)?;
        self.write_frame(&frame).await
    }

    pub(crate) async fn write_unframed(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        self.writes.write_all(&mut self.stream, bytes).await
    }

    pub(crate) async fn shutdown(&mut self) {
        self.stream.shutdown().await.ok();
    }

    pub fn enable_encryption(&mut self, key: &[u8; 16]) -> Result<(), CoreError> {
        if !self.decoder.read_buf_mut().is_empty() {
            return Err(CoreError::Auth(
                "client sent data ahead of the encryption switch".to_string(),
            ));
        }
        self.encrypt_cipher = Some(EncryptCipher::new(key));
        self.decrypt_cipher = Some(DecryptCipher::new(key));
        Ok(())
    }

    pub const fn set_compression(&mut self, threshold: i32) {
        self.decoder.set_compression(threshold);
        self.encoder.set_compression(threshold);
    }

    pub const fn compression_threshold(&self) -> Option<i32> {
        self.encoder.compression_threshold()
    }
}
