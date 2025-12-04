use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;
use async_trait::async_trait;
use infrarust_config::LogType;
use std::io::{self};
use std::time::Duration;
use tokio::time::timeout;
use tracing::instrument;
use tracing::{debug, warn};

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
        match message {
            MinecraftCommunication::Packet(data) => {
                actor.conn.write_packet(&data).await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Status client received shutdown"
                );
                if let Err(e) = actor.conn.close().await {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Error closing client connection: {:?}", e
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_external_client(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftClient<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        match data {
            PossibleReadValue::Packet(data) => {
                // Forward packet, ignoring send errors as they're expected in status mode
                let _ = actor
                    .server_sender
                    .send(MinecraftCommunication::Packet(data))
                    .await;
            }
            _ => {
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Client disconnected in status mode"
                );
                // Gracefully handle disconnection - don't try to notify server
            }
        }
        Ok(())
    }

    #[instrument(name = "status_client_init",skip(self, actor), fields(username = %actor.username))]
    async fn initialize_client(
        &self,
        actor: &mut MinecraftClient<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Initializing status client handler"
        );
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
        // This should never be called for status mode since we don't connect to backend
        if let PossibleReadValue::Packet(data) = data {
            // Ignore send errors in status mode
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
        // This is primarily for passthrough mode, less relevant for status
        if let MinecraftCommunication::Packet(data) = message
            && let Some(server_request) = actor.server_request.as_mut()
            && let Some(server_conn) = server_request.server_conn.as_mut()
        {
            server_conn.write_packet(&data).await?;
        }
        Ok(())
    }

    #[instrument(name = "status_server_init", skip(self, actor), fields(
        domain = %actor.server_request.as_ref().map(|r| r.proxied_domain.clone().unwrap_or_default()).unwrap_or_default()
    ))]
    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<StatusMessage>>,
    ) -> io::Result<()> {
        if let Some(request) = &actor.server_request {
            debug!(
                log_type = LogType::ProxyMode.as_str(),
                "Starting status mode for server request"
            );

            // Send the status response immediately
            if let Some(status) = &request.status_response {
                if actor
                    .client_sender
                    .send(MinecraftCommunication::Packet(status.clone()))
                    .await
                    .is_err()
                {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Client disconnected, cannot send status response"
                    );
                    return Ok(());
                }
            } else {
                warn!(
                    log_type = LogType::ProxyMode.as_str(),
                    "No status response available"
                );
                return Ok(());
            }

            // Wait for the ping packet with a timeout
            let ping_result = timeout(Duration::from_secs(5), actor.server_receiver.recv()).await;

            match ping_result {
                Ok(Some(MinecraftCommunication::Packet(packet))) => {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Received ping packet, sending back to client"
                    );
                    // Ignore error if client already disconnected
                    let _ = actor
                        .client_sender
                        .send(MinecraftCommunication::Packet(packet))
                        .await;
                }
                Ok(Some(_)) => {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Received non-packet message from client"
                    );
                }
                Ok(None) => {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Server receiver channel closed"
                    );
                }
                Err(_) => {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Timeout waiting for ping packet"
                    );
                }
            }

            // Always attempt to cleanly shut down the client connection
            let _ = actor
                .client_sender
                .send(MinecraftCommunication::Shutdown)
                .await;
            debug!(
                log_type = LogType::ProxyMode.as_str(),
                "Status response complete, status server actor shutting down"
            );
        } else {
            warn!(
                log_type = LogType::ProxyMode.as_str(),
                "No server request available for status mode"
            );
        }
        Ok(())
    }
}

impl ProxyMessage for StatusMessage {}

impl ProxyModeMessageType for StatusMode {
    type Message = StatusMessage;
}
