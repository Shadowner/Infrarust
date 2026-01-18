use infrarust_config::{LogType, ServerConfig};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};
use tokio::sync::mpsc;
use tracing::{Instrument, debug, error, instrument};

const UNKNOWN_PEER_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0));

use crate::Connection;
use crate::core::actors::supervisor::ActorSupervisor;
use crate::core::event::MinecraftCommunication;
use crate::proxy_modes::ClientProxyModeHandler;

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

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

    #[cfg(feature = "telemetry")]
    let peer_address = match actor.conn.peer_addr().await {
        Ok(addr) => addr,
        Err(e) => {
            debug!("Cannot get peer address: {:?}", e);

            // Ensure shutdown flag is set (Release ensures visibility to other threads)
            shutdown.store(true, Release);
            return;
        }
    };

    match proxy_mode.initialize_client(&mut actor).await {
        Ok(_) => {}
        Err(e) => {
            debug!("Error initializing client: {:?}", e);

            // Ensure connection is closed before returning
            let _ = actor.conn.close().await;
            return;
        }
    };

    // drop the span because it would be too long just for the connection processing
    drop(tracing::Span::current());

    let shutdown_flag = shutdown.clone();
    while !shutdown_flag.load(Acquire) {
        tokio::select! {
            Some(msg) = actor.supervisor_receiver.recv() => {
                actor.handle_supervisor_message(msg);
            }
            Some(msg) = actor.client_receiver.recv() => {
                if let MinecraftCommunication::Shutdown = msg {
                     shutdown_flag.store(true, Release);
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
                        shutdown_flag.store(true, Release);
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
                                debug!(log_type = LogType::TcpConnection.as_str(), "Error handling external client message: {:?}", e);
                                shutdown_flag.store(true, Release);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        debug!(log_type = LogType::TcpConnection.as_str(), "Client connection read error: {:?}", e);
                        shutdown_flag.store(true, Release);
                        break;
                    }
                }
            }
            else => {
                debug!(log_type = LogType::TcpConnection.as_str(), "All channels closed");
                shutdown_flag.store(true, Release);
                break;
            }
        }
    }

    debug!(
        log_type = LogType::TcpConnection.as_str(),
        "Shutting down client actor for {}",
        if actor.is_login && !actor.username.is_empty() {
            format!("user '{}'", actor.username)
        } else {
            "status request".to_string()
        }
    );

    let reason = if shutdown_flag.load(Acquire) {
        "clean_disconnect"
    } else {
        "unexpected_disconnect"
    };

    ActorSupervisor::global()
        .log_player_disconnect(actor.conn.session_id, reason)
        .await;

    #[cfg(feature = "telemetry")]
    if actor.is_login && !actor.username.is_empty() {
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

pub struct ClientHandlerConfig<T: Send + 'static> {
    pub server_sender: mpsc::Sender<MinecraftCommunication<T>>,
    pub client_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
    pub proxy_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
    pub conn: Connection,
    pub is_login: bool,
    pub username: String,
    pub shutdown: Arc<AtomicBool>,
    pub start_span: Option<tracing::Span>,
}
#[derive(Clone, Debug)]
pub struct MinecraftClientHandler {
    //TODO: establish a connection to talk to an actor
    _sender: mpsc::Sender<SupervisorMessage>,
    peer_addr: Option<std::net::SocketAddr>,
}

impl MinecraftClientHandler {
    //TODO: Refactor to remove the warning
    pub async fn new<T: Send + 'static>(config: ClientHandlerConfig<T>) -> Self {
        let span = tracing::Span::current();
        let (sender, receiver) = mpsc::channel(100);
        let peer_addr = config.conn.peer_addr().await.unwrap_or(UNKNOWN_PEER_ADDR);
        let actor = MinecraftClient::new(
            receiver,
            config.server_sender,
            config.client_receiver,
            config.conn,
            config.is_login,
            config.username,
        );

        if config.is_login {
            span.in_scope(|| {
                tokio::spawn(
                    start_minecraft_client_actor(actor, config.proxy_mode, config.shutdown)
                        .instrument(config.start_span.unwrap()),
                );
                Self {
                    _sender: sender,
                    peer_addr: Some(peer_addr),
                }
            })
        } else {
            tokio::spawn(
                start_minecraft_client_actor(actor, config.proxy_mode, config.shutdown)
                    .instrument(span),
            );
            Self {
                _sender: sender,
                peer_addr: Some(peer_addr),
            }
        }
    }

    pub async fn send_message(&self, message: SupervisorMessage) {
        let _ = self._sender.send(message).await;
    }

    /// Get the peer address of the client connection
    pub async fn get_peer_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.peer_addr
            .ok_or_else(|| std::io::Error::other("No peer address available"))
    }
}
