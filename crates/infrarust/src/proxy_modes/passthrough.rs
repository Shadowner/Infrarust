use super::{ClientProxyModeHandler, ProxyMessage, ProxyModeMessageType, ServerProxyModeHandler};
use crate::core::actors::client::MinecraftClient;
use crate::core::actors::server::MinecraftServer;
use crate::core::event::MinecraftCommunication;
use crate::network::connection::PossibleReadValue;
use async_trait::async_trait;
use std::io::{self};
use tracing::{debug, error};

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
                actor.conn.write_raw(&data).await?;
            }
            MinecraftCommunication::Shutdown => {
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
                // Don't fail immediately if send fails - server might be disconnecting naturally
                if actor
                    .server_sender
                    .send(MinecraftCommunication::RawData(data))
                    .await
                    .is_err()
                {
                    debug!("Server channel closed, client will close soon");
                }
            }
            _ => {
                debug!("Client disconnected, notifying server");
                // Ignore errors when sending shutdown - channel might already be closed
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
        debug!(
            log_type = "proxy_mode",
            "Initializing client passthrough proxy mode"
        );

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
                // Don't fail immediately if send fails - client might be disconnecting naturally
                if actor
                    .client_sender
                    .send(MinecraftCommunication::RawData(data))
                    .await
                    .is_err()
                {
                    debug!("Client channel closed");
                }
            }
            PossibleReadValue::Eof => {
                // Server disconnected unexpectedly - we need to force close everything
                debug!("Server EOF detected, initiating clean shutdown");

                // Explicitly notify client to close
                let _ = actor
                    .client_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await;

                // Close our end of the connection to the server too
                if let Some(server_request) = &mut actor.server_request {
                    if let Some(server_conn) = &mut server_request.server_conn {
                        if let Err(e) = server_conn.close().await {
                            debug!("Error closing server connection after EOF: {:?}", e);
                        }
                    }
                }

                // Return error to break the server actor loop
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Server disconnected",
                ));
            }
            _ => {
                debug!("Server disconnected, notifying client");
                // Ignore errors when sending shutdown - channel might already be closed
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
                let _ = actor
                    .server_request
                    .as_mut()
                    .unwrap()
                    .server_conn
                    .as_mut()
                    .unwrap()
                    .close()
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<PassthroughMessage>>,
    ) -> io::Result<()> {
        debug!(
            log_type = "proxy_mode",
            "Initializing server passthrough proxy mode"
        );
        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                for packet in server_request.read_packets.iter() {
                    server_conn.write_packet(packet).await?;
                }
                server_conn.enable_raw_mode();
            } else {
                error!(log_type = "proxy_mode", "Server connection is None");
            }
        } else {
            error!(log_type = "proxy_mode", "Server request is None");
        }
        Ok(())
    }
}
impl ProxyMessage for PassthroughMessage {}

impl ProxyModeMessageType for PassthroughMode {
    type Message = PassthroughMessage;
}
