use std::io::ErrorKind;

use bytes::BytesMut;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use infrarust_api::types::Component;
use infrarust_protocol::Packet;
use infrarust_protocol::codec::McBufWriteExt;
use infrarust_protocol::crypto::{DecryptCipher, EncryptCipher};
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::packets::config::CConfigDisconnect;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::pipeline::preauth_frame::refuse_oversized;
use crate::session::bounded_write::WriteGuard;
use crate::util::packets::encode_for_state;
use crate::util::text;

const READ_CHUNK: usize = 16 * 1024;
const MAX_HELD_BYTES: usize = 1024 * 1024;

pub struct ClientBridge {
    stream: TcpStream,
    writes: WriteGuard,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    encrypt_cipher: Option<EncryptCipher>,
    decrypt_cipher: Option<DecryptCipher>,
    pub protocol_version: ProtocolVersion,
    state: ConnectionState,
    awaiting_config_ack: bool,
}

impl ClientBridge {
    pub fn new(
        stream: TcpStream,
        buffered_data: BytesMut,
        protocol_version: ProtocolVersion,
    ) -> Self {
        let mut decoder = PacketDecoder::new();
        if !buffered_data.is_empty() {
            decoder.queue_bytes(&buffered_data);
        }

        Self {
            stream,
            writes: WriteGuard::new(),
            decoder,
            encoder: PacketEncoder::new(),
            encrypt_cipher: None,
            decrypt_cipher: None,
            protocol_version,
            state: ConnectionState::Login,
            awaiting_config_ack: false,
        }
    }

    pub(crate) const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    pub fn set_cancel(&mut self, cancel: CancellationToken) {
        self.writes.set_cancel(cancel);
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

    pub const fn set_state(&mut self, state: ConnectionState) {
        self.state = state;
    }

    pub const fn state(&self) -> ConnectionState {
        self.state
    }

    pub const fn begin_reconfiguration(&mut self) {
        self.state = ConnectionState::Config;
        self.awaiting_config_ack = true;
    }

    pub const fn reconfiguration_acknowledged(&mut self) {
        self.awaiting_config_ack = false;
    }

    pub const fn awaits_config_ack(&self) -> bool {
        self.awaiting_config_ack
    }

    pub async fn send_packet<P: Packet>(
        &mut self,
        packet: &P,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let frame = encode_for_state(packet, self.state, self.protocol_version, registry)?;
        self.write_frame(&frame).await
    }

    pub async fn close_with(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        let written = self.write_frame(frame).await;
        self.stream.shutdown().await.ok();
        written
    }

    pub async fn disconnect(
        &mut self,
        reason: &Component,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let version = self.protocol_version;
        match self.state {
            ConnectionState::Login => {
                let pkt = CLoginDisconnect {
                    reason: text::json_for(reason, version),
                };
                self.send_packet(&pkt, registry).await.ok();
            }
            ConnectionState::Config => {
                let reason = if text::uses_nbt(version, ConnectionState::Config) {
                    text::nbt_for(reason, version)
                } else {
                    let mut buf = Vec::new();
                    buf.write_string(&text::json_for(reason, version))?;
                    buf
                };
                let pkt = CConfigDisconnect { reason };
                self.send_packet(&pkt, registry).await.ok();
            }
            ConnectionState::Play => {
                let pkt = CDisconnect {
                    reason: text::encode_text_component(reason, version, ConnectionState::Play),
                };
                self.send_packet(&pkt, registry).await.ok();
            }
            _ => {}
        }
        self.stream.shutdown().await.ok();
        Ok(())
    }
}
