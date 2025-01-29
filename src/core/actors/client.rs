use log::{debug, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::{
    core::{
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    proxy_modes::ClientProxyModeHandler,
    Connection,
};

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
    pub supervisor_receiver: mpsc::Receiver<GatewayMessage>,
    pub conn: Connection,
    pub is_login: bool,
    pub username: String,
}

impl<T> MinecraftClient<T> {
    fn new(
        supervisor_receiver: mpsc::Receiver<GatewayMessage>,
        server_sender: mpsc::Sender<T>,
        client_receiver: mpsc::Receiver<T>,
        conn: Connection,
        is_login: bool,
        username: String,
    ) -> Self {
        Self {
            supervisor_receiver,
            server_sender,
            client_receiver,
            conn,
            is_login,
            username,
        }
    }

    fn handle_supervisor_message(&mut self, _message: GatewayMessage) {
        {}
    }
}
async fn start_minecraft_client_actor<T>(
    mut actor: MinecraftClient<MinecraftCommunication<T>>,
    proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
    shutdown: Arc<AtomicBool>,
) {
    debug!("Starting Minecraft Client Actor for ID");

    match proxy_mode.initialize_client(&mut actor).await {
        Ok(_) => {}
        Err(e) => {
            info!("Error initializing client: {:?}", e);
            return;
        }
    };

    let shutdown_flag = shutdown.clone();
    while !shutdown_flag.load(Ordering::SeqCst) {
        tokio::select! {
            Some(msg) = actor.supervisor_receiver.recv() => {
                actor.handle_supervisor_message(msg);
            }
            Some(msg) = actor.client_receiver.recv() => {
                if let MinecraftCommunication::Shutdown = msg {
                     shutdown_flag.store(true, Ordering::SeqCst);
                     actor.client_receiver.close();
                     let _ = actor.conn.close().await;
                }

                match proxy_mode.handle_internal_client(msg, &mut actor).await {
                    Ok(_) => {}
                    Err(e) => {
                        info!("Error handling internal client message: {:?}", e);
                        shutdown_flag.store(true, Ordering::SeqCst);
                    }
                };
            }
            Ok(read_value) = actor.conn.read() => {
                if let Err(e) = proxy_mode.handle_external_client(read_value, &mut actor).await {
                    info!("Error handling external client message: {:?}", e);
                    shutdown_flag.store(true, Ordering::SeqCst);
                }
            }
            else => {
                debug!("All channels closed");
                shutdown_flag.store(true, Ordering::SeqCst);
            }
        }
    }

    // Cleanup
    debug!("Shutting down client actor");

    actor.client_receiver.close();
}

#[derive(Clone)]
pub struct MinecraftClientHandler {
    //TODO: establish a connection to talk to an actor
    _sender: mpsc::Sender<GatewayMessage>,
}

impl MinecraftClientHandler {
    pub fn new<T: Send + 'static>(
        server_sender: mpsc::Sender<MinecraftCommunication<T>>,
        client_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
        proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        conn: Connection,
        is_login: bool,
        username: String,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        // TODO: Implement better supervisor handling
        let (sender, receiver) = mpsc::channel(100);
        let actor = MinecraftClient::new(
            receiver,
            server_sender,
            client_receiver,
            conn,
            is_login,
            username,
        );

        tokio::spawn(start_minecraft_client_actor(actor, proxy_mode, shutdown));

        Self { _sender: sender }
    }
}
