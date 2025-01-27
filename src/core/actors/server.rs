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
    proxy_modes::{self, ServerProxyModeHandler},
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

pub struct MinecraftServer<T> {
    pub server_request: Option<ServerResponse>,
    pub gateway_receiver: mpsc::Receiver<GatewayMessage>,

    pub client_sender: mpsc::Sender<T>,
    pub server_receiver: mpsc::Receiver<T>,

    pub is_login: bool,
}

impl<T> MinecraftServer<T> {
    fn new(
        gateway_receiver: mpsc::Receiver<GatewayMessage>,

        client_sender: mpsc::Sender<T>,
        server_receiver: mpsc::Receiver<T>,

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

    fn set_server_request(&mut self, request: ServerResponse) {
        self.server_request = Some(request);
    }
}

async fn start_minecraft_server_actor<T>(
    mut actor: MinecraftServer<MinecraftCommunication<T>>,
    oneshot: oneshot::Receiver<ServerResponse>,
    proxy_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
) where
    T: Send + 'static,
{
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

    actor.server_request = Some(request);

    if let Some(request) = &actor.server_request {
        match proxy_mode.initialize_server(&mut actor).await {
            Ok(_) => {}
            Err(e) => {
                warn!("Failed to initialize server proxy mode: {:?}", e);
                client_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await
                    .unwrap();
                debug!("Shutting down Minecraft Server Actor");
                return;
            }
        };
    }


    debug!("Starting Minecraft Server Actor for ID");
    loop {
        if let Some(server_request) = &mut actor.server_request {
            if server_request.status_response.is_some() {
                debug!("Server request is a status response");
                break;
            }

            if let Some(server_conn) = &mut server_request.server_conn {
                tokio::select! {
                    Some(msg) = actor.gateway_receiver.recv() => {
                        actor.handle_gateway_message(msg);
                    }
                    Some(msg) = actor.server_receiver.recv() => {
                        let _ = proxy_mode.handle_internal_server(msg, &mut actor).await;
                    }
                    Ok(read_value) = server_conn.read() => {
                        let _ = proxy_mode.handle_external_server(read_value, &mut actor).await;
                    }
                    else => break,
                }
            } else {
                warn!("Server connection is None L114");
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
    pub fn new<T: Send + 'static>(
        client_sender: mpsc::Sender<MinecraftCommunication<T>>,
        server_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
        is_login: bool,
        request_server: oneshot::Receiver<ServerResponse>,
        proxy_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);

        let actor = MinecraftServer::new(receiver, client_sender, server_receiver, is_login);
        tokio::spawn(start_minecraft_server_actor(
            actor,
            request_server,
            proxy_mode,
        ));

        Self {
            sender_to_actor: sender,
        }
    }

    pub async fn send_message(&self, message: GatewayMessage) {
        let _ = self.sender_to_actor.send(message).await;
    }
}
