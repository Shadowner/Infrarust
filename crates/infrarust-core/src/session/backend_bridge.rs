use std::ops::{Deref, DerefMut};

use tokio::net::TcpStream;

use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::SLoginStart;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::auth::game_profile::offline_uuid;
use crate::error::CoreError;
use crate::forwarding::domain_rewrite::rewrite_handshake;
use crate::pipeline::types::HandshakeData;
use crate::session::framed_stream::FramedStream;

const ONLINE_BACKEND: &str =
    "the backend is in online mode; intercepted modes need online-mode=false";

pub struct BackendBridge {
    framed: FramedStream,
    server_address: Option<infrarust_config::ServerAddress>,
}

impl Deref for BackendBridge {
    type Target = FramedStream;

    fn deref(&self) -> &FramedStream {
        &self.framed
    }
}

impl DerefMut for BackendBridge {
    fn deref_mut(&mut self) -> &mut FramedStream {
        &mut self.framed
    }
}

impl BackendBridge {
    pub fn new(stream: TcpStream, protocol_version: ProtocolVersion) -> Self {
        Self {
            framed: FramedStream::new(stream, protocol_version),
            server_address: None,
        }
    }

    #[must_use]
    pub fn with_server_address(mut self, address: infrarust_config::ServerAddress) -> Self {
        self.server_address = Some(address);
        self
    }

    pub const fn server_address(&self) -> Option<&infrarust_config::ServerAddress> {
        self.server_address.as_ref()
    }

    pub(crate) fn online_mode_refusal(&self) -> CoreError {
        let backend = self
            .server_address
            .as_ref()
            .map_or_else(String::new, ToString::to_string);
        tracing::warn!(%backend, "{ONLINE_BACKEND}");
        CoreError::Auth(ONLINE_BACKEND.to_string())
    }

    pub async fn send_initial_packets(
        &mut self,
        handshake_data: &HandshakeData,
        server_config: &ServerConfig,
    ) -> Result<(), CoreError> {
        let handshake_bytes = rewrite_handshake(handshake_data, server_config)?;
        self.write_unframed(&handshake_bytes).await?;

        for raw in handshake_data.raw_packets.iter().skip(1) {
            self.write_unframed(raw).await?;
        }
        Ok(())
    }

    pub async fn send_handshake(&mut self, handshake: &SHandshake) -> Result<(), CoreError> {
        let mut payload = Vec::new();
        handshake.encode(&mut payload, self.protocol_version())?;
        self.queue_raw(0x00, &payload)?;
        self.flush().await
    }

    pub async fn send_login_start(
        &mut self,
        username: &str,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let login_start = SLoginStart {
            name: username.to_string(),
            uuid: Some(offline_uuid(username)),
            profile_key: None,
        };
        self.send_packet(&login_start, registry).await
    }
}
