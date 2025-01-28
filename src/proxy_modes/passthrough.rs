use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;
use async_trait::async_trait;
use log::{debug, error};
use std::io::{self};

pub struct PassthroughMode;

#[derive(Debug)]
pub enum PassthroughMessage {
    RawData(Vec<u8>),
}

#[async_trait]
impl ClientProxyModeHandler<MinecraftCommunication<PassthroughMessage>> for PassthroughMode {
    async fn handle_internal_client(
        &self,
        message: MinecraftCommunication<PassthroughMessage>,
        actor: &mut MinecraftClient<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::RawData(data) => {
                debug!(
                    "Received raw data from client (PassthroughMode) with length: {}",
                    data.len()
                );
                actor.conn.write_raw(&data).await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!("Shutting down client (Received Shutdown message)");
                actor.conn.close().await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        match data {
            PossibleReadValue::Raw(data) => {
                let _ = actor
                    .server_sender
                    .send(MinecraftCommunication::RawData(data))
                    .await;
            }
            _ => {
                debug!("Shutting down client ");
                let _ = actor
                    .server_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await;
            }
        }
        Ok(())
    }

    async fn initialize_client(
        &self,
        actor: &mut MinecraftClient<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing client passthrough proxy mode");

        actor.conn.enable_raw_mode();
        Ok(())
    }
}

#[async_trait]
impl ServerProxyModeHandler<MinecraftCommunication<PassthroughMessage>> for PassthroughMode {
    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        match data {
            PossibleReadValue::Raw(data) => {
                let _ = actor
                    .client_sender
                    .send(MinecraftCommunication::RawData(data))
                    .await;
            }
            _ => {
                debug!("Shutting down server");
                let _ = actor
                    .client_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await;
            }
        }

        Ok(())
    }

    async fn handle_internal_server(
        &self,
        message: MinecraftCommunication<PassthroughMessage>,
        actor: &mut MinecraftServer<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        match message {
            MinecraftCommunication::RawData(data) => {
                actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .write_raw(&data)
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
        actor: &mut MinecraftServer<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing server passthrough proxy mode");
        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                for packet in server_request.read_packets.iter() {
                    server_conn.write_packet(packet).await?;
                }
                server_conn.enable_raw_mode();
            } else {
                error!("Server connection is None");
            }
        } else {
            error!("Server request is None");
        }
        Ok(())
    }
}
impl ProxyMessage for PassthroughMessage {}

impl ProxyModeMessageType for PassthroughMode {
    type Message = PassthroughMessage;
}
