use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, info, instrument};

use crate::core::actors::supervisor::ActorSupervisor;
use crate::telemetry::TELEMETRY;
use crate::{
    Connection,
    core::{config::ServerConfig, event::MinecraftCommunication},
    proxy_modes::ClientProxyModeHandler,
};

use super::supervisor::SupervisorMessage;

pub enum ClientEvent {
    ConfigurationUpdate {
        key: String,
        configuration: Box<ServerConfig>,
    },
    Shutdown,
}

pub struct MinecraftClient<T> {
    pub server_sender: mpsc::Sender<T>,
    pub client_receiver: mpsc::Receiver<T>,
    pub supervisor_receiver: mpsc::Receiver<SupervisorMessage>,
    pub conn: Connection,
    pub is_login: bool,
    pub username: String,
}

impl<T> MinecraftClient<T> {
    fn new(
        supervisor_receiver: mpsc::Receiver<SupervisorMessage>,
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

    fn handle_supervisor_message(&mut self, _message: SupervisorMessage) {
        {}
    }
}

#[instrument(skip(actor, proxy_mode, shutdown), fields(
    is_login = actor.is_login,
    name = %actor.username
))]
async fn start_minecraft_client_actor<T>(
    mut actor: MinecraftClient<MinecraftCommunication<T>>,
    proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
    shutdown: Arc<AtomicBool>,
) {
    debug!("Starting Minecraft Client Actor for ID");

    let peer_address = match actor.conn.peer_addr().await {
        Ok(addr) => addr,
        Err(e) => {
            info!("Cannot get peer address: {:?}", e);

            // Ensure shutdown flag is set
            shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
            return;
        }
    };

    match proxy_mode.initialize_client(&mut actor).await {
        Ok(_) => {}
        Err(e) => {
            info!("Error initializing client: {:?}", e);

            // Ensure connection is closed before returning
            let _ = actor.conn.close().await;
            return;
        }
    };
    // drop the span because it would be too long just for the connection processing
    drop(tracing::Span::current());

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
                     if let Err(e) = actor.conn.close().await {
                         error!("Error closing client connection during shutdown: {:?}", e);
                     }
                     break;
                }

                match proxy_mode.handle_internal_client(msg, &mut actor).await {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Error handling internal client message: {:?}", e);
                        shutdown_flag.store(true, Ordering::SeqCst);
                        break;
                    }
                };
            }
            read_result = actor.conn.read() => {
                match read_result {
                    Ok(read_value) => {
                        match proxy_mode.handle_external_client(read_value, &mut actor).await {
                            Ok(_) => {}
                            Err(e) => {
                                debug!("Error handling external client message: {:?}", e);
                                shutdown_flag.store(true, Ordering::SeqCst);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Client connection read error: {:?}", e);
                        shutdown_flag.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
            else => {
                debug!("All channels closed");
                shutdown_flag.store(true, Ordering::SeqCst);
                break;
            }
        }
    }

    // Enhanced cleanup with more details on disconnect reason
    debug!("Shutting down client actor for user '{}'", actor.username);

    // Record telemetry for logins with more context
    if actor.is_login && !actor.username.is_empty() {
        let reason = if shutdown_flag.load(Ordering::SeqCst) {
            "clean_disconnect"
        } else {
            "unexpected_disconnect"
        };

        match ActorSupervisor::global() {
            supervisor => {
                supervisor
                            .log_player_disconnect(actor.conn.session_id, reason)
                            .await;
            }
        }

        TELEMETRY.record_connection_end(&peer_address.to_string(), reason, actor.conn.session_id);
    }

    // Close the client connection
    if let Err(e) = actor.conn.close().await {
        debug!("Error during final client connection close: {:?}", e);
    }

    let _ = actor
        .server_sender
        .send(MinecraftCommunication::Shutdown)
        .await;
}

#[derive(Clone)]
pub struct MinecraftClientHandler {
    //TODO: establish a connection to talk to an actor
    _sender: mpsc::Sender<SupervisorMessage>,
}

impl MinecraftClientHandler {
    //TODO: Refactor to remove the warning
    #[allow(clippy::too_many_arguments)]
    pub fn new<T: Send + 'static>(
        server_sender: mpsc::Sender<MinecraftCommunication<T>>,
        client_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
        proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        conn: Connection,
        is_login: bool,
        username: String,
        shutdown: Arc<AtomicBool>,
        start_span: Option<tracing::Span>,
    ) -> Self {
        let span = tracing::Span::current();
        let (sender, receiver) = mpsc::channel(100);

        let actor = MinecraftClient::new(
            receiver,
            server_sender,
            client_receiver,
            conn,
            is_login,
            username,
        );

        if is_login {
            span.in_scope(|| {
                tokio::spawn(
                    start_minecraft_client_actor(actor, proxy_mode, shutdown)
                        .instrument(start_span.unwrap()),
                );
                Self { _sender: sender }
            })
        } else {
            tokio::spawn(
                start_minecraft_client_actor(actor, proxy_mode, shutdown).instrument(span),
            );
            Self { _sender: sender }
        }
    }

    pub async fn send_message(&self, message: SupervisorMessage) {
        let _ = self._sender.send(message).await;
    }
}
