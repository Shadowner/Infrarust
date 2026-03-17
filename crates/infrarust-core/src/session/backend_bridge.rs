//! Backend-side bridge for intercepted proxy modes.
//!
//! Wraps the backend TCP stream with packet codec and optional compression.
//! No encryption in Phase 2A (Offline/ClientOnly backends run in offline mode).

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder, PacketFrame};
use infrarust_protocol::packets::login::SLoginStart;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::auth::game_profile::offline_uuid;
use crate::error::CoreError;
use crate::pipeline::types::HandshakeData;
use crate::util::domain_rewrite::rewrite_handshake;

/// The backend side of a proxied connection.
///
/// Can be replaced during a server switch (future Phase 4+).
pub struct BackendBridge {
    stream: TcpStream,
    decoder: PacketDecoder,
    encoder: PacketEncoder,
    /// Current protocol state.
    pub state: ConnectionState,
    read_buf: BytesMut,
}

impl BackendBridge {
    /// Creates a new backend bridge from an established connection.
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            decoder: PacketDecoder::new(),
            encoder: PacketEncoder::new(),
            state: ConnectionState::Login,
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
            .get_packet_id::<P>(self.state, P::direction(), self.protocol_version())
            .ok_or_else(|| {
                CoreError::Auth(format!("no packet ID for {} in {:?}", P::NAME, self.state,))
            })?;

        let mut payload = Vec::new();
        packet.encode(&mut payload, self.protocol_version())?;

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

    /// Returns a placeholder protocol version.
    /// In practice, the version is tracked by the handler and passed where needed.
    #[allow(clippy::unused_self)] // Kept as method for future per-connection version tracking
    const fn protocol_version(&self) -> ProtocolVersion {
        // Default to latest; the registry handles version-specific lookups
        ProtocolVersion::V1_21
    }
}
