use async_trait::async_trait;
use log::{debug, error, info};
use std::io::{self};

use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;

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
                    debug!("Received Compression packet {:?}", data);
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
                info!("Unhandled message: {:?}", message);
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
                    //TODO: Better handle phase of actors (playing_phase, login_phase)
                    if data.id == 0x03 && !server_conn.is_compressing() {
                        debug!("Received Compression packet srv {:?}", data);
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
                debug!("Received packet from server: {:?}", data);
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

    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<OfflineMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing server offline proxy mode");
        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                for packet in server_request.read_packets.iter() {
                    server_conn.write_packet(packet).await?;
                }
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
