use std::sync::Arc;

use log::{debug, info, warn};
use rsa::rand_core::le;
use tokio::sync::{mpsc, oneshot};

use crate::{
    core::{
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    network::connection::PossibleReadValue,
    server::{self, gateway, ServerRequest, ServerResponse},
    ServerConnection,
};

pub enum ServerEvent {
    ConfigurationUpdate {
        key: String,
        configuration: ServerConfig,
    },
    Shutdown,
}

struct MinecraftServer {
    server_request: Option<ServerResponse>,
    gateway_receiver: mpsc::Receiver<GatewayMessage>,

    client_sender: mpsc::Sender<MinecraftCommunication>,
    server_receiver: mpsc::Receiver<MinecraftCommunication>,
    is_login: bool,
}

impl MinecraftServer {
    fn new(
        gateway_receiver: mpsc::Receiver<GatewayMessage>,

        client_sender: mpsc::Sender<MinecraftCommunication>,
        server_receiver: mpsc::Receiver<MinecraftCommunication>,
        is_login: bool,
    ) -> Self {
        Self {
            gateway_receiver,
            server_request: None,
            client_sender,
            server_receiver,
            is_login,
        }
    }

    fn handle_gateway_message(&mut self, message: GatewayMessage) {
        match message {
            _ => {}
        }
    }

    async fn handle_message(&mut self, message: MinecraftCommunication) {
        match message {
            MinecraftCommunication::Packet(packet) => {
                debug!("Received packet from: {:?}", packet);
                // We are sure that the server_request is Some because we only receive packets after a server request
                if let Some(server_conn) = &mut self.server_request.as_mut().unwrap().server_conn {
                    let _ = server_conn.write_packet(&packet).await;
                }
            }
            MinecraftCommunication::RawData(data) => {
                debug!("Received raw data: {:?}", data);
                // We are sure that the server_request is Some because we only receive packets after a server request

                if let Some(server_conn) = &mut self.server_request.as_mut().unwrap().server_conn {
                    let _ = server_conn.write_raw(&data).await;
                }
            }
            MinecraftCommunication::Shutdown => {
                info!("Shutting down Minecraft Server Actor");
                if let Some(server_conn) = &mut self.server_request.as_mut().unwrap().server_conn {
                    let _ = server_conn.close().await;
                }
            }
            _ => {}
        }
    }

    fn set_server_request(&mut self, request: ServerResponse) {
        self.server_request = Some(request);
        // Handle any initialization needed with the new server request
    }
}

async fn start_minecraft_server_actor(
    mut actor: MinecraftServer,
    oneshot: oneshot::Receiver<ServerResponse>,
) {
    debug!("Starting Minecraft Server Actor");
    let client_sender = actor.client_sender.clone();

    // Wait for the server request in a separate task to not block message processing
    let request = match oneshot.await {
        Ok(req) => req,
        _ => {
            debug!("Failed to receive server request");
            client_sender
                .send(MinecraftCommunication::Shutdown)
                .await
                .unwrap();
            debug!("Shutting down Minecraft Server Actor");
            return;
        }
    };

    debug!("Received server request: {:?}", request.proxied_domain);

    let packets = request.read_packets.clone();

    if request.status_response.is_some() {
        debug!("Sending status response to client");
        let _ = actor
            .client_sender
            .send(MinecraftCommunication::Packet(
                request.status_response.unwrap(),
            ))
            .await;

        let ping_packet = match actor.server_receiver.recv().await {
            Some(MinecraftCommunication::Packet(packet)) => packet,
            _ => {
                debug!("Failed to receive ping packet from server");
                client_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await
                    .unwrap();
                return;
            }
        };

        debug!("Received ping packet from server: {:?}", ping_packet);
        actor
            .client_sender
            .send(MinecraftCommunication::Packet(ping_packet))
            .await
            .unwrap();

        client_sender
            .send(MinecraftCommunication::Shutdown)
            .await
            .unwrap();
        return;
    }

    actor.set_server_request(request);

    debug!("Sending packets to server : {:?}", packets);
    for packet in packets {
        debug!("Sending packet to server: {:?}", packet);
        actor
            .handle_message(MinecraftCommunication::Packet(packet))
            .await;
    }
    debug!("Finished sending packets to server");

    loop {
        if let Some(server_request) = &mut actor.server_request {
            if let Some(server_conn) = &mut server_request.server_conn {
                tokio::select! {
                    Some(msg) = actor.gateway_receiver.recv() => {
                        actor.handle_gateway_message(msg);
                    }
                    Some(msg) = actor.server_receiver.recv() => {
                        actor.handle_message(msg).await;
                    }
                    Ok(read_value) = server_conn.read() => {
                        match read_value {
                            PossibleReadValue::Packet(packet) => {
                                debug!("Received packet from Server: {:?}", packet);
                                let _ = actor.client_sender.send(MinecraftCommunication::Packet(packet)).await;

                            },
                            PossibleReadValue::Raw(data) => {
                                debug!("Received raw data from Server: {:?}", data.len());
                                let _ = actor.client_sender.send(MinecraftCommunication::RawData(data)).await;
                            }
                        }
                    }
                    else => break,
                }
            } else {
                warn!("Server connection is None");
                break;
            }
        } else {
            warn!("Server request is None");
            break;
        }
    }
}

#[derive(Clone)]
pub struct MinecraftServerHandler {
    sender_to_actor: mpsc::Sender<GatewayMessage>,
}

impl MinecraftServerHandler {
    pub fn new(
        client_sender: mpsc::Sender<MinecraftCommunication>,
        server_receiver: mpsc::Receiver<MinecraftCommunication>,
        is_login: bool,
        request_server: oneshot::Receiver<ServerResponse>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(100);

        let actor = MinecraftServer::new(receiver, client_sender, server_receiver, is_login);
        tokio::spawn(start_minecraft_server_actor(actor, request_server));

        Self {
            sender_to_actor: sender,
        }
    }

    pub fn send_message(&self, message: GatewayMessage) {
        self.sender_to_actor.send(message);
    }
}
