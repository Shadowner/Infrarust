use std::{
    fmt::Debug,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use infrarust_config::{LogType, ServerConfig};
use tokio::sync::{mpsc, oneshot};
use tracing::{Instrument, debug, debug_span, warn};

use crate::{
    core::event::{GatewayMessage, MinecraftCommunication},
    network::connection::PossibleReadValue,
    proxy_modes::ServerProxyModeHandler,
    server::ServerResponse,
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

pub enum ServerEvent {
    ConfigurationUpdate {
        key: String,
        configuration: Box<ServerConfig>,
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

    fn handle_gateway_message(&mut self, _message: GatewayMessage) -> io::Result<()> {
        Ok(())
    }
    pub fn server_conn_mut(
        &mut self,
    ) -> io::Result<&mut crate::network::connection::ServerConnection> {
        self.server_request
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "No server request"))?
            .server_conn
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "No server connection"))
    }
}

async fn start_minecraft_server_actor<T>(
    mut actor: MinecraftServer<MinecraftCommunication<T>>,
    oneshot: oneshot::Receiver<ServerResponse>,
    proxy_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
    shutdown: Arc<AtomicBool>,
) where
    T: Send + std::fmt::Debug + 'static,
{
    let main_span = debug_span!("minecraft_server_actor", is_login = actor.is_login);
    async {
        let client_sender = actor.client_sender.clone();

        debug!("Waiting for server request from oneshot channel");
        let request = match oneshot.await {
            Ok(req) => {
                debug!("Received server request");

                if actor.is_login
                    && let Some(domain) = &req.proxied_domain
                    && let Some(server_conn) = &req.server_conn
                {
                    debug!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Server domain for connection {}: {}",
                        server_conn.session_id, domain
                    );
                }

                req
            }
            Err(e) => {
                debug!(log_type = LogType::TcpConnection.as_str(), "Failed to receive server request: {:?}", e);
                if actor.is_login
                    && client_sender
                        .send(MinecraftCommunication::Shutdown)
                        .await
                        .is_err()
                {
                    debug!(log_type = LogType::TcpConnection.as_str(), "Client channel already closed during server initialization");
                }
                actor.server_receiver.close();
                debug!(log_type = LogType::TcpConnection.as_str(), "Shutting down Minecraft Server Actor due to missing request");
                return;
            }
        };

        actor.server_request = Some(request);

        if actor.server_request.is_some() {
            debug!(log_type = LogType::TcpConnection.as_str(), "Initializing server proxy mode");
            match proxy_mode.initialize_server(&mut actor).await {
                Ok(_) => debug!(log_type = LogType::TcpConnection.as_str(), "Server proxy mode initialized successfully"),
                Err(e) => {
                    warn!(log_type = LogType::TcpConnection.as_str(), "Failed to initialize server proxy mode: {:?}", e);
                    if client_sender
                        .send(MinecraftCommunication::Shutdown)
                        .await
                        .is_err()
                    {
                        debug!(log_type = LogType::TcpConnection.as_str(), "Client channel already closed");
                    }
                    actor.server_receiver.close();
                    debug!(log_type = LogType::TcpConnection.as_str(), "Shutting down Minecraft Server Actor");
                    return;
                }
            };
        } else {
            warn!(log_type = LogType::TcpConnection.as_str(), "Server request is None");
        }

        debug!(log_type = LogType::TcpConnection.as_str(), "Starting Minecraft Server Actor main loop");
        while !shutdown.load(Ordering::SeqCst) {
            if actor
                .server_request
                .as_ref()
                .is_some_and(|req| req.status_response.is_some())
            {
                debug!(log_type = LogType::TcpConnection.as_str(), "Returning because Actor is for a Status Check");
                return;
            }

            let server_conn_available = actor
                .server_request
                .as_ref()
                .is_some_and(|req| req.server_conn.is_some());

            if !server_conn_available {
                warn!(log_type = LogType::TcpConnection.as_str(), "Server connection is None");
                break;
            }

            let shutdown_flag = shutdown.clone();

            // Add a timeout to each select iteration to avoid getting stuck if it ever happen
            let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(5));

            tokio::select! {
                Some(msg) = actor.gateway_receiver.recv() => {
                    if let Err(e) = actor.handle_gateway_message(msg) {
                        warn!("Error handling gateway message: {:?}", e);
                        shutdown_flag.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                Some(msg) = actor.server_receiver.recv() => {
                    if let MinecraftCommunication::Shutdown = &msg {
                        debug!("Shutting down server (Received Shutdown message)");
                        // Close the server connection
                        if let Some(server_request) = &mut actor.server_request
                            && let Some(server_conn) = &mut server_request.server_conn
                            && let Err(e) = server_conn.close().await
                        {
                            warn!("Error closing server connection: {:?}", e);
                        }
                        actor.server_receiver.close();
                        shutdown_flag.store(true, Ordering::SeqCst);
                        break;
                    } else if let Err(e) = proxy_mode.handle_internal_server(msg, &mut actor).await {
                        warn!(log_type = LogType::TcpConnection.as_str(), "Error handling internal server message: {:?}", e);
                        shutdown_flag.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                read_result = read_from_server(&mut actor.server_request) => {
                    match read_result {
                        Ok(read_value) => {
                            if let Err(e) = proxy_mode.handle_external_server(read_value, &mut actor).await {
                                warn!(log_type = LogType::TcpConnection.as_str(), "Error handling external server message: {:?}", e);
                                shutdown_flag.store(true, Ordering::SeqCst);
                                break;
                            }
                        }
                        Err(e) => {
                            warn!(log_type = LogType::TcpConnection.as_str(), "Error reading from server: {:?}", e);
                            shutdown_flag.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
                _ = timeout => {
                    // If we hit the timeout, just continue to the next iteration
                    debug!(log_type = LogType::TcpConnection.as_str(), "Server actor select timeout - continuing");
                }
                else => {
                    debug!(log_type = LogType::TcpConnection.as_str(), "All channels closed");
                    shutdown_flag.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }

        debug!(log_type = LogType::TcpConnection.as_str(), "Exiting Minecraft Server Actor main loop");

        if let Some(server_request) = &mut actor.server_request
            && let Some(server_conn) = &mut server_request.server_conn
            && let Err(e) = server_conn.close().await
        {
            debug!("Error during final server connection close: {:?}", e);
        }

        let _ = client_sender.send(MinecraftCommunication::Shutdown).await;

        actor.server_receiver.close();

        #[cfg(feature = "telemetry")]
        if actor.is_login
            && actor.server_request.is_some()
            && actor.server_request.as_ref().unwrap().server_conn.is_some()
        {
            TELEMETRY.update_player_count(
                -1,
                actor
                    .server_request
                    .as_ref()
                    .unwrap()
                    .initial_config
                    .config_id
                    .as_str(),
                actor
                    .server_request
                    .as_ref()
                    .unwrap()
                    .server_conn
                    .as_ref()
                    .unwrap()
                    .session_id,
                "",
            );
        }
        debug!("Shutting down server actor");
        if client_sender
            .send(MinecraftCommunication::Shutdown)
            .await
            .is_err()
        {
            debug!("Client channel already closed during server shutdown");
        }
        actor.server_receiver.close();
    }.instrument(main_span).await;
}

async fn read_from_server(
    server_request: &mut Option<ServerResponse>,
) -> Result<PossibleReadValue, std::io::Error> {
    if let Some(req) = server_request {
        if let Some(conn) = &mut req.server_conn {
            conn.read().await
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No server connection",
            ))
        }
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "No server request",
        ))
    }
}

#[derive(Clone, Debug)]
pub struct MinecraftServerHandler {
    sender_to_actor: mpsc::Sender<GatewayMessage>,
}

impl MinecraftServerHandler {
    pub fn new<T: Send + Debug + 'static>(
        client_sender: mpsc::Sender<MinecraftCommunication<T>>,
        server_receiver: mpsc::Receiver<MinecraftCommunication<T>>,
        is_login: bool,
        request_server: oneshot::Receiver<ServerResponse>,
        proxy_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
        shutdown: Arc<AtomicBool>,
        start_span: Option<tracing::Span>,
    ) -> Self {
        let span = tracing::Span::current();
        let (sender, receiver) = mpsc::channel(64);

        let actor = MinecraftServer::new(receiver, client_sender, server_receiver, is_login);

        if is_login {
            span.in_scope(|| {
                tokio::spawn(
                    start_minecraft_server_actor(
                        actor,
                        request_server,
                        proxy_mode,
                        shutdown,
                        //We'are sure that in is_login the start_span exist
                    )
                    .instrument(start_span.unwrap()),
                );

                Self {
                    sender_to_actor: sender,
                }
            })
        } else {
            tokio::spawn(
                start_minecraft_server_actor(actor, request_server, proxy_mode, shutdown)
                    .instrument(span),
            );

            Self {
                sender_to_actor: sender,
            }
        }
    }

    pub async fn send_message(&self, message: GatewayMessage) {
        let _ = self.sender_to_actor.send(message).await;
    }
}
