use log::{debug, info};
use serde::de;
use std::collections::HashMap;
use tokio::sync::mpsc;

use crate::{
    core::{
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    network::connection::PossibleReadValue,
    Connection,
};

use super::{server::ServerEvent, Actor};

pub enum ClientEvent {
    ConfigurationUpdate {
        key: String,
        configuration: ServerConfig,
    },
    Shutdown,
}

struct MinecraftClient {
    server_sender: mpsc::Sender<MinecraftCommunication>,
    client_receiver: mpsc::Receiver<MinecraftCommunication>,
    gateway_receiver: mpsc::Receiver<GatewayMessage>,

    conn: Connection,
    is_login: bool,
}

impl MinecraftClient {
    fn new(
        gateway_receiver: mpsc::Receiver<GatewayMessage>,

        server_sender: mpsc::Sender<MinecraftCommunication>,
        client_receiver: mpsc::Receiver<MinecraftCommunication>,
        conn: Connection,
        is_login: bool,
    ) -> Self {
        Self {
            gateway_receiver,
            server_sender,
            client_receiver,
            conn,
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
            MinecraftCommunication::Shutdown => {
                info!("Shutting down Minecraft Client Actor");
                self.client_receiver.close();
                let _ = self.conn.close();
            }
            MinecraftCommunication::Packet(packet) => {
                debug!("Received packet: {:?}", packet);
                let _ = self.conn.write_packet(&packet).await;
                debug!("Sent packet to client");
            }
            MinecraftCommunication::RawData(data) => {
                debug!("Received raw data: {:?}", data);
                let _ = self.conn.write_raw(&data);
            }
            _ => {}
        }
    }
}
async fn start_minecraft_client_actor(mut actor: MinecraftClient) {
    debug!("Starting Minecraft Client Actor for ID");

    loop {
        tokio::select! {
            Some(msg) = actor.gateway_receiver.recv() => {
                actor.handle_gateway_message(msg);
            }
            Some(msg) = actor.client_receiver.recv() => {
                actor.handle_message(msg).await;
            }
            read_value = actor.conn.read() => {
                        match read_value {
                            Ok(PossibleReadValue::Packet(packet)) => {
                                debug!("Received packet from Client: {:?}", packet);
                                let _ = actor.server_sender.send(MinecraftCommunication::Packet(packet)).await;
                            },

                            Ok(PossibleReadValue::Raw(data)) => {
                                debug!("Received raw data from Client: {:?}", data.len());
                                let _ = actor.server_sender.send(MinecraftCommunication::RawData(data)).await;
                            }
                            Err(e) => {
                                let _ = actor.server_sender.send(MinecraftCommunication::Shutdown).await;
                                break;
                            }
                        }
            }
        }
    }
}

#[derive(Clone)]
pub struct MinecraftClientHandler {
    receiver: mpsc::Sender<GatewayMessage>,
}

impl MinecraftClientHandler {
    pub fn new(
        server_sender: mpsc::Sender<MinecraftCommunication>,
        client_receiver: mpsc::Receiver<MinecraftCommunication>,
        conn: Connection,
        is_login: bool,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        let actor = MinecraftClient::new(receiver, server_sender, client_receiver, conn, is_login);

        tokio::spawn(start_minecraft_client_actor(actor));

        Self { receiver: sender }
    }

    pub fn send_message(&self, message: GatewayMessage) {
        let _ = self.receiver.send(message);
    }
}
