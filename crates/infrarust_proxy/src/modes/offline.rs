use async_trait::async_trait;
use std::io::{self};
use tracing::{debug, debug_span, error, info, instrument, Instrument};

use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::{
    core::{
        actors::client::MinecraftClient, actors::server::MinecraftServer,
        event::MinecraftCommunication,
    },
    network::connection::PossibleReadValue,
};

pub struct OfflineMode;

#[derive(Debug)]
pub enum OfflineMessage {}

#[async_trait]
impl ClientProxyModeHandler<MinecraftCommunication<OfflineMessage>> for OfflineMode {
    async fn handle_internal_client(
        &self,
        message: MinecraftCommunication<OfflineMessage>,
        actor: &mut MinecraftClient<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::Packet(data) => {
                if data.id == 0x03 && !actor.conn.is_compressing() {
                    debug!("Received Compression packet");
                    actor.conn.write_packet(&data).await?;
                    actor.conn.enable_compression(256);
                    return Ok(());
                }
                actor.conn.write_packet(&data).await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!("Shutting down client (Received Shutdown message)");
                actor.conn.close().await?;
            }
            _ => {
                info!("Unhandled message");
            }
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        if let PossibleReadValue::Packet(data) = data {
            let _ = actor
                .server_sender
                .send(MinecraftCommunication::Packet(data))
                .await;
        }
        Ok(())
    }

    #[instrument(name = "offline_client_init", skip(self, _actor), fields(username = %_actor.username))]
    async fn initialize_client(
        &self,
        _actor: &mut MinecraftClient<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing client offline proxy mode");
        Ok(())
    }
}

#[async_trait]
impl ServerProxyModeHandler<MinecraftCommunication<OfflineMessage>> for OfflineMode {
    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        if let Some(request) = &mut actor.server_request {
            if let Some(server_conn) = &mut request.server_conn {
                if let PossibleReadValue::Packet(data) = data {
                    if data.id == 0x03 && !server_conn.is_compressing() {
                        debug!("Received Compression packet srv");
                        server_conn.enable_compression(256);
                    }

                    let _ = actor
                        .client_sender
                        .send(MinecraftCommunication::Packet(data))
                        .await;
                }
            }
        }
        Ok(())
    }

    async fn handle_internal_server(
        &self,
        message: MinecraftCommunication<OfflineMessage>,
        actor: &mut MinecraftServer<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::Packet(data) => {
                debug!("Received packet from server");
                actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .write_packet(&data)
                    .await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!("Shutting down server (Received Shutdown message)");
                actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .close()
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    #[instrument(name = "offline_server_init", skip(self, actor), fields(
        domain = %actor.server_request.as_ref().map(|r| r.initial_config.domains.join(", ")).unwrap_or_else(|| "unknown".to_string())
    ))]
    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing server offline proxy mode");

        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                let span = debug_span!("initialize_connection");
                async {
                    for packet in server_request.read_packets.iter() {
                        server_conn.write_packet(packet).await?;
                    }
                    Ok::<(), io::Error>(())
                }
                .instrument(span)
                .await?;
            } else {
                error!("Server connection is None");
            }
        } else {
            error!("Server request is None");
        }
        Ok(())
    }
}

impl ProxyMessage for OfflineMessage {}

impl ProxyModeMessageType for OfflineMode {
    type Message = OfflineMessage;
}
