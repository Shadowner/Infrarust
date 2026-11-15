use std::ops::{Deref, DerefMut};
use std::time::Duration;

use tokio::net::TcpStream;

use infrarust_config::ServerConfig;
use infrarust_protocol::Packet;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CEncryptionRequest, CLoginDisconnect, CLoginPluginRequest, CLoginSuccess, CSetCompression,
    SLoginPluginResponse, SLoginStart,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::auth::game_profile::offline_uuid;
use crate::error::CoreError;
use crate::forwarding::domain_rewrite::rewrite_handshake;
use crate::pipeline::types::HandshakeData;
use crate::session::framed_stream::FramedStream;
use crate::session::kick::BackendKick;
use crate::session::login_messages::LoginMessages;

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

    pub async fn consume_backend_login(
        &mut self,
        registry: &PacketRegistry,
        version: ProtocolVersion,
        velocity_ctx: Option<(&crate::forwarding::ForwardingData, &[u8])>,
        messages: &LoginMessages<'_>,
    ) -> Result<(), CoreError> {
        let encryption_request = registry.get_packet_id::<CEncryptionRequest>(version);
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let frame = self
                    .read_frame()
                    .await?
                    .ok_or(CoreError::ConnectionClosed)?;

                if Some(frame.id) == encryption_request {
                    return Err(self.online_mode_refusal());
                }

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

                        if packet.as_any().downcast_ref::<CLoginDisconnect>().is_some() {
                            return Err(CoreError::BackendKick(Box::new(BackendKick::new(
                                frame,
                                ConnectionState::Login,
                                version,
                            ))));
                        }

                        if let Some(request) = packet.as_any().downcast_ref::<CLoginPluginRequest>()
                        {
                            if crate::forwarding::velocity::is_velocity_request(request) {
                                if let Some((fwd_data, secret)) = velocity_ctx {
                                    let response =
                                        crate::forwarding::velocity::build_velocity_response(
                                            request, fwd_data, secret,
                                        )?;
                                    self.send_packet(&response, registry).await?;
                                    tracing::info!("velocity forwarding applied (auto-detected)");
                                    continue;
                                }
                                tracing::debug!(
                                    "backend requested velocity forwarding but no secret is \
                                     configured — responding 'not understood'"
                                );
                            } else {
                                let response = messages.answer(&frame, request).await;
                                self.send_packet(&response, registry).await?;
                                continue;
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
