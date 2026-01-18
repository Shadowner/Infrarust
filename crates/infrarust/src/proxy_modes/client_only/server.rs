use std::io;

use async_trait::async_trait;
use infrarust_config::LogType;
use infrarust_protocol::types::VarInt;
use tracing::{debug, error};

use crate::{
    core::{actors::server::MinecraftServer, event::MinecraftCommunication},
    network::{connection::PossibleReadValue, packet::PacketCodec},
    proxy_modes::{
        ServerProxyModeHandler,
        client_only::{ClientOnlyMessage, prepare_server_handshake},
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
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Received packet from server: {:?}", data
                );
                actor.server_conn_mut()?.write_packet(&data).await?;
                actor.server_conn_mut()?.flush().await?;
            }
            MinecraftCommunication::Shutdown => {
                debug!(
                    log_type = LogType::ProxyMode.as_str(),
                    "Shutting down server (Received Shutdown message)"
                );
                let _ = actor.server_conn_mut()?.close().await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn initialize_server(
        &self,
        actor: &mut MinecraftServer<MinecraftCommunication<ClientOnlyMessage>>,
    ) -> io::Result<()> {
        debug!(
            log_type = LogType::ProxyMode.as_str(),
            "Initializing server offline proxy mode"
        );
        let mut server_initialised = false;

        if let Some(request) = actor.server_request.as_mut()
            && let Some(conn) = request.server_conn.as_mut()
        {
            let client_handshake = &request.read_packets[0];
            let server_addr = conn.peer_addr().await;
            let login_start = &request.read_packets[1];

            // REFACTO : unwrapped it but might do something better
            let server_handshake =
                prepare_server_handshake(client_handshake, &server_addr.unwrap())?;
            conn.write_packet(&server_handshake).await?;
            conn.write_packet(login_start).await?;
            conn.flush().await?;

            while !server_initialised {
                match conn.read_packet().await? {
                    packet if packet.id == 0x03 => {
                        // Set Compression
                        let threshold = packet.decode::<VarInt>();
                        if threshold.is_err() {
                            error!(
                                log_type = LogType::ProxyMode.as_str(),
                                "Failed to decode compression threshold"
                            );
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Failed to decode compression threshold",
                            ));
                        }
                        let threshold = threshold.unwrap();

                        if threshold.0 >= 0 {
                            debug!(
                                log_type = LogType::ProxyMode.as_str(),
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
                                        log_type = LogType::ProxyMode.as_str(),
                                        "Failed to send ServerThreshold message to client: {}", e
                                    );
                                }
                            }
                        }
                    }
                    packet if packet.id == 0x02 => {
                        // Ignore server's Login Success, we'll use ours
                        debug!(
                            log_type = LogType::ProxyMode.as_str(),
                            "Received Login Success from server, Server Initialised"
                        );
                        server_initialised = true;
                    }
                    packet => {
                        debug!(
                            log_type = LogType::ProxyMode.as_str(),
                            "Received packet {:?} from server", packet
                        );
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
                    error!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Failed to send ServerReady message to client: {}", e
                    );
                }
            };

            if let Some(msg) = actor.server_receiver.recv().await {
                match msg {
                    MinecraftCommunication::CustomData(
                        ClientOnlyMessage::ClientLoginAcknowledged(packet),
                    ) => {
                        debug!(
                            log_type = LogType::ProxyMode.as_str(),
                            "Server received ClientLoginAcknowledged message"
                        );
                        conn.write_packet(&packet).await?;
                        conn.flush().await?;
                    }
                    _ => {
                        error!(
                            log_type = LogType::ProxyMode.as_str(),
                            "Unexpected message waited Acknowledged got : {:?}", msg
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
