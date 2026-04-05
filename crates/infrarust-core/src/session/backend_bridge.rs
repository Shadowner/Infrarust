//! Backend-side bridge for intercepted proxy modes.
//!
//! Wraps the backend TCP stream with packet codec and optional compression.

use std::time::Duration;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression, SLoginPluginResponse,
    SLoginStart,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::auth::game_profile::offline_uuid;
use crate::error::CoreError;
use crate::pipeline::types::HandshakeData;
use crate::util::domain_rewrite::rewrite_handshake;

/// The backend side of a proxied connection.
///
/// Can be replaced during a server switch (Phase 4+).
pub struct BackendBridge {
    stream: TcpStream,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    /// Current protocol state.
    pub state: ConnectionState,
    /// Protocol version of this connection.
    pub protocol_version: ProtocolVersion,
    read_buf: BytesMut,
}

impl BackendBridge {
    pub fn new(stream: TcpStream, protocol_version: ProtocolVersion) -> Self {
        Self {
            stream,
            decoder: PacketDecoder::new(),
            encoder: PacketEncoder::new(),
            state: ConnectionState::Login,
            protocol_version,
            read_buf: BytesMut::with_capacity(4096),
        }
    }

    /// Reads the next packet frame from the backend.
    ///
    /// Returns `Ok(None)` on clean disconnect (EOF).
    ///
    /// # Errors
    /// Returns `CoreError` on I/O or protocol decode errors.
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

            self.decoder.queue_bytes(&self.read_buf[..n]);
        }
    }

    /// Writes an encoded packet frame to the backend.
    ///
    /// # Errors
    /// Returns `CoreError` on I/O or encoding errors.
    pub async fn write_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        self.encoder.append_frame(frame)?;
        let data = self.encoder.take();
        self.stream.write_all(&data).await?;
        Ok(())
    }

    /// Encodes and sends a typed packet to the backend.
    ///
    /// # Errors
    /// Returns `CoreError` if packet ID lookup fails or I/O errors occur.
    pub async fn send_packet<P: Packet>(
        &mut self,
        packet: &P,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let packet_id = registry
            .get_packet_id::<P>(self.state, P::direction(), self.protocol_version)
            .ok_or_else(|| {
                CoreError::Auth(format!("no packet ID for {} in {:?}", P::NAME, self.state,))
            })?;

        let mut payload = Vec::new();
        packet.encode(&mut payload, self.protocol_version)?;

        self.encoder.append_raw(packet_id, &payload)?;
        let data = self.encoder.take();
        self.stream.write_all(&data).await?;
        Ok(())
    }

    /// Activates packet compression with the given threshold.
    pub const fn set_compression(&mut self, threshold: i32) {
        self.decoder.set_compression(threshold);
        self.encoder.set_compression(threshold);
    }

    /// Changes the protocol state.
    pub const fn set_state(&mut self, state: ConnectionState) {
        self.state = state;
    }

    /// Sends handshake + login start packets to the backend.
    ///
    /// Applies domain rewrite according to the server config.
    /// Used by `OfflineHandler` where the client's original login is forwarded.
    ///
    /// # Errors
    /// Returns `CoreError` on handshake rewrite or I/O errors.
    pub async fn send_initial_packets(
        &mut self,
        handshake_data: &HandshakeData,
        server_config: &ServerConfig,
    ) -> Result<(), CoreError> {
        // Write (possibly rewritten) handshake
        let handshake_bytes = rewrite_handshake(handshake_data, server_config)?;
        self.stream.write_all(&handshake_bytes).await?;

        // Forward remaining raw packets (login start, etc.) as-is
        for raw in handshake_data.raw_packets.iter().skip(1) {
            self.stream.write_all(raw).await?;
        }

        self.stream.flush().await?;
        Ok(())
    }

    /// Sends handshake + login start with an offline UUID to the backend.
    ///
    /// Used by `ClientOnlyHandler` where the proxy authenticates the client
    /// and then connects to the backend in offline mode.
    ///
    /// # Errors
    /// Returns `CoreError` on handshake rewrite, encoding, or I/O errors.
    pub async fn send_initial_packets_offline(
        &mut self,
        handshake_data: &HandshakeData,
        server_config: &ServerConfig,
        username: &str,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let version = handshake_data.protocol_version;

        // Write (possibly rewritten) handshake
        let handshake_bytes = rewrite_handshake(handshake_data, server_config)?;
        self.stream.write_all(&handshake_bytes).await?;

        // Build and send login start with offline UUID
        let uuid = offline_uuid(username);
        let login_start = SLoginStart {
            name: username.to_string(),
            uuid: Some(uuid),
            signature_data: None,
        };

        let packet_id = registry
            .get_packet_id::<SLoginStart>(ConnectionState::Login, Direction::Serverbound, version)
            .unwrap_or(0x00);

        let mut payload = Vec::new();
        login_start.encode(&mut payload, version)?;

        self.encoder.append_raw(packet_id, &payload)?;
        let data = self.encoder.take();
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;

        Ok(())
    }

    pub async fn send_handshake_and_login(
        &mut self,
        handshake: &SHandshake,
        username: &str,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let version = self.protocol_version;

        let mut handshake_payload = Vec::new();
        handshake.encode(&mut handshake_payload, version)?;
        self.encoder.append_raw(0x00, &handshake_payload)?;
        let handshake_data = self.encoder.take();
        self.stream.write_all(&handshake_data).await?;

        let uuid = offline_uuid(username);
        let login_start = SLoginStart {
            name: username.to_string(),
            uuid: Some(uuid),
            signature_data: None,
        };

        let packet_id = registry
            .get_packet_id::<SLoginStart>(ConnectionState::Login, Direction::Serverbound, version)
            .unwrap_or(0x00);

        let mut payload = Vec::new();
        login_start.encode(&mut payload, version)?;

        self.encoder.append_raw(packet_id, &payload)?;
        let data = self.encoder.take();
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;

        Ok(())
    }

    /// Consumes the backend's login response without forwarding to the client.
    ///
    /// Reads `SetCompression` (activates compression on backend) and `LoginSuccess`
    /// (consumed without forwarding). Returns error on `LoginDisconnect`.
    ///
    /// After this call, the backend is ready for the next phase
    /// (Config for 1.20.2+, or Play for older versions).
    ///
    /// # Errors
    /// Returns `CoreError` on I/O errors, protocol errors, or backend rejection.
    pub async fn consume_backend_login(
        &mut self,
        registry: &PacketRegistry,
        version: ProtocolVersion,
        velocity_ctx: Option<(&crate::forwarding::ForwardingData, &[u8])>,
    ) -> Result<(), CoreError> {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let frame = self
                    .read_frame()
                    .await?
                    .ok_or(CoreError::ConnectionClosed)?;

                let decoded = registry.decode_frame(
                    &frame,
                    ConnectionState::Login,
                    Direction::Clientbound,
                    version,
                )?;

                match decoded {
                    DecodedPacket::Typed { packet, .. } => {
                        if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                            self.set_compression(set_comp.threshold.0);
                            tracing::debug!(
                                threshold = set_comp.threshold.0,
                                "backend compression activated"
                            );
                            continue;
                        }

                        if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                            if version.less_than(ProtocolVersion::V1_20_2) {
                                self.set_state(ConnectionState::Play);
                            }
                            tracing::debug!("consumed backend LoginSuccess");
                            break;
                        }

                        if let Some(disconnect) = packet.as_any().downcast_ref::<CLoginDisconnect>()
                        {
                            return Err(CoreError::Rejected(format!(
                                "backend refused login: {}",
                                disconnect.reason
                            )));
                        }

                        if let Some(request) = packet.as_any().downcast_ref::<CLoginPluginRequest>()
                        {
                            if crate::forwarding::velocity::is_velocity_request(request) {
                                if let Some((fwd_data, secret)) = velocity_ctx {
                                    let response =
                                        crate::forwarding::velocity::build_velocity_response(
                                            request, fwd_data, secret,
                                        );
                                    self.send_packet(&response, registry).await?;
                                    tracing::info!("velocity forwarding applied (auto-detected)");
                                    continue;
                                }
                                tracing::debug!(
                                    "backend requested velocity forwarding but no secret is \
                                     configured — responding 'not understood'"
                                );
                            } else {
                                tracing::debug!(
                                    channel = %request.channel,
                                    "responding 'not understood' to LoginPluginRequest"
                                );
                            }

                            let response = SLoginPluginResponse {
                                message_id: request.message_id,
                                successful: false,
                                data: Vec::new(),
                            };
                            self.send_packet(&response, registry).await?;
                            continue;
                        }
                    }
                    DecodedPacket::Opaque { id, .. } => {
                        tracing::debug!(id, "ignoring opaque packet during backend login");
                    }
                }
            }

            Ok(())
        })
        .await
        .map_err(|_| CoreError::Rejected("backend login timed out".to_string()))?
    }
}
