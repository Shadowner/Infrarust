use std::io;

use async_trait::async_trait;
use log::{debug, error};

use crate::{
    core::{actors::server::MinecraftServer, event::MinecraftCommunication},
    network::{connection::PossibleReadValue, packet::PacketCodec},
    protocol::types::VarInt,
    proxy_modes::{
        client_only::{prepare_server_handshake, ClientOnlyMessage},
        ServerProxyModeHandler,
    },
};

use super::ClientOnlyMode;

#[async_trait]
impl ServerProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>> for ClientOnlyMode {
    async fn handle_external_server(
        &self,
        data: PossibleReadValue,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
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
        message: MinecraftCommunication<ClientOnlyMessage>,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
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
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        debug!("Initializing server offline proxy mode");
        let mut server_initialised = false;

        if let Some(request) = actor.server_request.as_mut() {
            if let Some(conn) = request.server_conn.as_mut() {
                let client_handshake = &request.read_packets[0];
                let server_addr = conn.peer_addr().await?;
                let login_start = &request.read_packets[1];

                let server_handshake = prepare_server_handshake(client_handshake, &server_addr)?;
                conn.write_packet(&server_handshake).await?;
                conn.write_packet(login_start).await?;

                while !server_initialised {
                    match conn.read_packet().await? {
                        packet if packet.id == 0x03 => {
                            // Set Compression
                            let threshold = packet.decode::<VarInt>()?;
                            if threshold.0 >= 0 {
                                debug!(
                                    "Received compression config from server, threshold: {}",
                                    threshold.0
                                );

                                conn.enable_compression(threshold.0);
                                match actor
                                    .client_sender
                                    .send(MinecraftCommunication::CustomData(
                                        ClientOnlyMessage::ServerThreshold(threshold),
                                    ))
                                    .await
                                {
                                    Ok(_) => {}
                                    Err(e) => {
                                        error!(
                                            "Failed to send ServerThreshold message to client: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                        packet if packet.id == 0x02 => {
                            // Ignore server's Login Success, we'll use ours
                            debug!("Received Login Success from server, Server Initialised");
                            server_initialised = true;
                        }
                        packet => {
                            debug!("Received packet {:?} from server", packet);
                        }
                    }
                }

                match actor
                    .client_sender
                    .send(MinecraftCommunication::CustomData(
                        ClientOnlyMessage::ServerReady(),
                    ))
                    .await
                {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Failed to send ServerReady message to client: {}", e);
                    }
                };

                if let Some(msg) = actor.server_receiver.recv().await {
                    match msg {
                        MinecraftCommunication::CustomData(
                            ClientOnlyMessage::ClientLoginAknowledged(packet),
                        ) => {
                            debug!("Server received ClientLoginAknowledged message");
                            conn.write_packet(&packet).await?;
                        }
                        _ => {
                            error!("Unexpected message waited Aknowledge got : {:?}", msg);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
