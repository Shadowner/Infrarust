//! Client-side bridge for intercepted proxy modes.
//!
//! Wraps the client TCP stream with packet codec, optional encryption
//! (AES-128-CFB8), and optional compression. Survives backend death
//! to support future server switch / limbo.

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

/// The client side of a proxied connection.
///
/// Handles reading/writing Minecraft packet frames from/to the client,
/// with optional encryption and compression.
pub struct ClientBridge {
    stream: TcpStream,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    encrypt_cipher: Option<EncryptCipher>,
    decrypt_cipher: Option<DecryptCipher>,
    /// The client's protocol version.
    pub protocol_version: ProtocolVersion,
    state: ConnectionState,
    read_buf: BytesMut,
}

impl ClientBridge {
    /// Creates a new client bridge.
    ///
    /// `buffered_data` contains bytes already read by the pipeline middlewares
    /// (handshake, login start). These are fed to the decoder first.
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
            decoder,
            encoder: PacketEncoder::new(),
            encrypt_cipher: None,
            decrypt_cipher: None,
            protocol_version,
            state: ConnectionState::Login,
            read_buf: BytesMut::with_capacity(4096),
        }
    }

    /// Reads the next packet frame from the client.
    ///
    /// Returns `Ok(None)` on clean disconnect (EOF).
    /// Handles decryption if encryption is active.
    pub async fn read_frame(&mut self) -> Result<Option<PacketFrame>, CoreError> {
        loop {
            if let Some(frame) = self.decoder.try_next_frame()? {
                return Ok(Some(frame));
            }

            self.read_buf.resize(4096, 0);
            let n = self.stream.read(&mut self.read_buf).await?;
            if n == 0 {
                return Ok(None);
            }

            if let Some(cipher) = &mut self.decrypt_cipher {
                cipher.decrypt(&mut self.read_buf[..n]);
            }
            self.decoder.queue_bytes(&self.read_buf[..n]);
        }
    }

    /// Writes an encoded packet frame to the client.
    ///
    /// Handles encryption if active.
    pub async fn write_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        self.encoder.append_frame(frame)?;
        let mut data = self.encoder.take();
        if let Some(cipher) = &mut self.encrypt_cipher {
            cipher.encrypt(&mut data);
        }
        self.stream.write_all(&data).await?;
        Ok(())
    }

    /// Enables AES-128-CFB8 encryption with the given shared secret.
    ///
    /// In Minecraft, the IV equals the key. Both encrypt and decrypt
    /// ciphers are created from the same 16-byte key.
    pub fn enable_encryption(&mut self, key: &[u8; 16]) {
        self.encrypt_cipher = Some(EncryptCipher::new(key));
        self.decrypt_cipher = Some(DecryptCipher::new(key));
    }

    /// Activates packet compression with the given threshold.
    pub fn set_compression(&mut self, threshold: i32) {
        self.decoder.set_compression(threshold);
        self.encoder.set_compression(threshold);
    }

    /// Changes the protocol state (Login → Config → Play).
    pub fn set_state(&mut self, state: ConnectionState) {
        self.state = state;
    }

    /// Returns the current protocol state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Encodes and sends a typed packet to the client.
    pub async fn send_packet<P: Packet>(
        &mut self,
        packet: &P,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let packet_id = registry
            .get_packet_id::<P>(self.state, P::direction(), self.protocol_version)
            .ok_or_else(|| {
                CoreError::Auth(format!(
                    "no packet ID for {} in {:?}/{:?}",
                    P::NAME,
                    self.state,
                    self.protocol_version
                ))
            })?;

        let mut payload = Vec::new();
        packet.encode(&mut payload, self.protocol_version)?;

        self.encoder.append_raw(packet_id, &payload)?;
        let mut data = self.encoder.take();
        if let Some(cipher) = &mut self.encrypt_cipher {
            cipher.encrypt(&mut data);
        }
        self.stream.write_all(&data).await?;
        Ok(())
    }

    /// Sends a disconnect packet and shuts down the connection.
    ///
    /// Uses the correct packet type and encoding for the current state and version:
    /// - Login: `CLoginDisconnect` (JSON string)
    /// - Config: `CConfigDisconnect` (JSON for 1.20.2, NBT for 1.20.3+)
    /// - Play: `CDisconnect` (JSON for <1.20.3, NBT for 1.20.3+)
    pub async fn disconnect(
        &mut self,
        reason: &str,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let json = serde_json::json!({"text": reason}).to_string();
        match self.state {
            ConnectionState::Login => {
                let pkt = CLoginDisconnect { reason: json };
                self.send_packet(&pkt, registry).await.ok();
            }
            ConnectionState::Config => {
                let reason_bytes = if self.protocol_version.less_than(ProtocolVersion::V1_20_3) {
                    // 1.20.2: Chat component as VarInt-prefixed JSON string
                    let mut buf = Vec::new();
                    buf.write_string(&json)?;
                    buf
                } else {
                    // 1.20.3+: NBT text component
                    encode_nbt_text_component(reason)
                };
                let pkt = CConfigDisconnect {
                    reason: reason_bytes,
                };
                self.send_packet(&pkt, registry).await.ok();
            }
            ConnectionState::Play => {
                let reason_bytes = if self.protocol_version.less_than(ProtocolVersion::V1_20_3) {
                    // JSON bytes — CDisconnect.encode() adds the VarInt length prefix
                    json.into_bytes()
                } else {
                    // NBT text component
                    encode_nbt_text_component(reason)
                };
                let pkt = CDisconnect {
                    reason: reason_bytes,
                };
                self.send_packet(&pkt, registry).await.ok();
            }
            _ => {}
        }
        self.stream.shutdown().await.ok();
        Ok(())
    }
}

/// Encodes a plain-text message as an NBT text component for 1.20.3+.
///
/// Produces a TAG_Compound with a "text" TAG_String field, using
/// standard NBT encoding (empty root name).
fn encode_nbt_text_component(text: &str) -> Vec<u8> {
    let text_bytes = text.as_bytes();
    #[allow(clippy::cast_possible_truncation)]
    let text_len = text_bytes.len() as u16;

    let mut buf = Vec::with_capacity(3 + 7 + text_bytes.len() + 1);
    buf.push(0x0A); // TAG_Compound (root)
    buf.extend_from_slice(&[0x00, 0x00]); // empty root name
    buf.push(0x08); // TAG_String
    buf.extend_from_slice(&4u16.to_be_bytes()); // field name "text" (length 4)
    buf.extend_from_slice(b"text");
    buf.extend_from_slice(&text_len.to_be_bytes()); // string value length
    buf.extend_from_slice(text_bytes);
    buf.push(0x00); // TAG_End
    buf
}
