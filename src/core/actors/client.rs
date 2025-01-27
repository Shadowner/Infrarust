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
    proxy_modes::ClientProxyModeHandler,
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

pub struct MinecraftClient<T> {
    pub server_sender: mpsc::Sender<T>,
    pub client_receiver: mpsc::Receiver<T>,
    pub gateway_receiver: mpsc::Receiver<GatewayMessage>,
    pub conn: Connection,
    pub is_login: bool,
}

impl<T> MinecraftClient<T> {
    fn new(
        gateway_receiver: mpsc::Receiver<GatewayMessage>,
        server_sender: mpsc::Sender<T>,
        client_receiver: mpsc::Receiver<T>,
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
}
async fn start_minecraft_client_actor<T>(
    mut actor: MinecraftClient<MinecraftCommunication<T>>,
    proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
) {
    debug!("Starting Minecraft Client Actor for ID");

    match proxy_mode.initialize_client(&mut actor).await {
        Ok(_) => {}
        Err(e) => {
            info!("Error initializing client: {:?}", e);
            return;
        }
    };

    loop {
        tokio::select! {
            Some(msg) = actor.gateway_receiver.recv() => {
                actor.handle_gateway_message(msg);
            }
            Some(msg) = actor.client_receiver.recv() => {
                match proxy_mode.handle_internal_client(msg, &mut actor).await {
                    Ok(_) => {}
                    Err(e) => {
                        info!("Error handling internal client message: {:?}", e);
                        actor.server_sender.send(MinecraftCommunication::Shutdown).await.unwrap();
                        return;
                    }
                };
            } 
            Ok(read_value) = actor.conn.read() => {
                let _ = proxy_mode.handle_external_client(read_value, &mut actor).await;
            }
            else => {
                info!("Client actor shutting down");
                break;
            }
        }
    }
}

#[derive(Clone)]
pub struct MinecraftClientHandler {
    sender: mpsc::Sender<GatewayMessage>,
}
impl MinecraftClientHandler {
    pub fn new<T: Send + 'static>(
        server_sender: mpsc::Sender<MinecraftCommunication<T>>,
        client_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
        proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        conn: Connection,
        is_login: bool,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        let actor = MinecraftClient::new(receiver, server_sender, client_receiver, conn, is_login);

        tokio::spawn(start_minecraft_client_actor(actor, proxy_mode));

        Self { sender }
    }
}
