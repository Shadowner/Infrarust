use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, instrument, Instrument};

use crate::network::connection::PossibleReadValue;
use crate::telemetry::{Direction, TELEMETRY};
use crate::{
    core::{
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    proxy_modes::ClientProxyModeHandler,
    Connection,
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
    let start_time = std::time::Instant::now();
    debug!("Starting Minecraft Client Actor for ID");
    let peer_address = actor.conn.peer_addr().await.unwrap();
    match proxy_mode.initialize_client(&mut actor).await {
        Ok(_) => {}
        Err(e) => {
            info!("Error initializing client: {:?}", e);
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
    if actor.is_login {
        TELEMETRY.record_connection_end(
            &peer_address.to_string(),
            start_time.elapsed().as_secs_f64(),
            "normal_disconnect",
            actor.conn.session_id,
        );
    }
    let _ = actor.conn.close().await;
}

#[derive(Clone)]
pub struct MinecraftClientHandler {
    //TODO: establish a connection to talk to an actor
    _sender: mpsc::Sender<SupervisorMessage>,
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
                //We'are sure that in is_login the start_span exist
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
