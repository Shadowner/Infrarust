use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;
use async_trait::async_trait;
use std::io::{self};
use tracing::debug;
use tracing::instrument;

pub struct StatusMode;

#[derive(Debug)]
pub enum StatusMessage {}

#[async_trait]
impl ClientProxyModeHandler<MinecraftCommunication<StatusMessage>> for StatusMode {
    async fn handle_internal_client(
        &self,
        message: MinecraftCommunication<StatusMessage>,
        actor: &mut MinecraftClient<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let MinecraftCommunication::Packet(data) = message {
            actor.conn.write_packet(&data).await?;
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let PossibleReadValue::Packet(data) = data {
            let _ = actor
                .server_sender
                .send(MinecraftCommunication::Packet(data))
                .await;
        }
        Ok(())
    }

    #[instrument(name = "status_client_init",skip(self, actor), fields(username = %actor.username))]
    async fn initialize_client(
        &self,
        actor: &mut MinecraftClient<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ServerProxyModeHandler<MinecraftCommunication<StatusMessage>> for StatusMode {
    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let PossibleReadValue::Packet(data) = data {
            let _ = actor
                .client_sender
                .send(MinecraftCommunication::Packet(data))
                .await;
        }

        Ok(())
    }

    async fn handle_internal_server(
        &self,
        message: MinecraftCommunication<StatusMessage>,
        actor: &mut MinecraftServer<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let MinecraftCommunication::Packet(data) = message {
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
        Ok(())
    }

    #[instrument(name = "status_server_init", skip(self, actor), fields(
        domain = %actor.server_request.as_ref().map(|r| r.initial_config.domains.join(", ")).unwrap_or_else(|| "unknown".to_string())
    ))]
    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let Some(request) = &actor.server_request {
            debug!("Starting status mode for server request");
            let _ = actor
                .client_sender
                .send(MinecraftCommunication::Packet(
                    request.status_response.clone().unwrap(),
                ))
                .await;

            let ping_packet = match actor.server_receiver.recv().await {
                Some(MinecraftCommunication::Packet(packet)) => packet,
                _ => {
                    debug!("Failed to receive ping packet from server");
                    let _ = actor
                        .client_sender
                        .send(MinecraftCommunication::Shutdown)
                        .await;
                    return Ok(());
                }
            };

            debug!("Received ping packet from server: {:?}", ping_packet);
            actor
                .client_sender
                .send(MinecraftCommunication::Packet(ping_packet))
                .await
                .unwrap();

            debug!("Sending status response to client");
            let _ = actor
                .client_sender
                .send(MinecraftCommunication::Packet(
                    request.status_response.clone().unwrap(),
                ))
                .await;

            actor
                .client_sender
                .send(MinecraftCommunication::Shutdown)
                .await
                .unwrap();
            debug!("Shutting down Minecraft Server Actor Status Mode");
        }
        Ok(())
    }
}
impl ProxyMessage for StatusMessage {}

impl ProxyModeMessageType for StatusMode {
    type Message = StatusMessage;
}
