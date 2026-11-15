use std::ops::{Deref, DerefMut};

use bytes::BytesMut;
use tokio::net::TcpStream;

use infrarust_api::types::Component;
use infrarust_protocol::codec::McBufWriteExt;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::CConfigDisconnect;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::session::framed_stream::FramedStream;
use crate::util::text;

pub struct ClientBridge {
    framed: FramedStream,
    awaiting_config_ack: bool,
}

impl Deref for ClientBridge {
    type Target = FramedStream;

    fn deref(&self) -> &FramedStream {
        &self.framed
    }
}

impl DerefMut for ClientBridge {
    fn deref_mut(&mut self) -> &mut FramedStream {
        &mut self.framed
    }
}

impl ClientBridge {
    pub fn new(
        stream: TcpStream,
        buffered_data: BytesMut,
        protocol_version: ProtocolVersion,
    ) -> Self {
        let mut framed = FramedStream::new(stream, protocol_version);
        framed.queue_received(&buffered_data);
        Self {
            framed,
            awaiting_config_ack: false,
        }
    }

    pub const fn begin_reconfiguration(&mut self) {
        self.framed.set_state(ConnectionState::Config);
        self.awaiting_config_ack = true;
    }

    pub const fn reconfiguration_acknowledged(&mut self) {
        self.awaiting_config_ack = false;
    }

    pub const fn awaits_config_ack(&self) -> bool {
        self.awaiting_config_ack
    }

    pub async fn close_with(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        let written = self.write_frame(frame).await;
        self.shutdown().await;
        written
    }

    pub async fn disconnect(
        &mut self,
        reason: &Component,
        registry: &PacketRegistry,
    ) -> Result<(), CoreError> {
        let version = self.protocol_version();
        match self.state() {
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
        self.shutdown().await;
        Ok(())
    }
}
