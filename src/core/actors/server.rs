use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc,
    },
};

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, instrument, warn, Instrument};

use crate::{
    core::{
        actors::server,
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    network::connection::PossibleReadValue,
    proxy_modes::ServerProxyModeHandler,
    server::ServerResponse,
    telemetry::{Direction, TELEMETRY},
};

use super::supervisor::SupervisorMessage;

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
}

#[instrument(skip(actor, oneshot, proxy_mode, shutdown), fields(
    is_login = actor.is_login
))]
async fn start_minecraft_server_actor<T>(
    mut actor: MinecraftServer<MinecraftCommunication<T>>,
    oneshot: oneshot::Receiver<ServerResponse>,
    proxy_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
    shutdown: Arc<AtomicBool>,
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
            actor.server_receiver.close();
            debug!("Shutting down Minecraft Server Actor");
            return;
        }
    };

    actor.server_request = Some(request);

    // Just to ensure that the initialize_server has the request
    if actor.server_request.is_some() {
        match proxy_mode.initialize_server(&mut actor).await {
            Ok(_) => {}
            Err(_e) => {
                warn!("Failed to initialize server proxy mode: {:?}", _e);
                client_sender
                    .send(MinecraftCommunication::Shutdown)
                    .await
                    .unwrap();
                actor.server_receiver.close();
                debug!("Shutting down Minecraft Server Actor");
                return;
            }
        };
    } else {
        warn!("Server request is None");
    }

    debug!("Starting Minecraft Server Actor for ID");
    while !shutdown.load(Ordering::SeqCst) {
        if let Some(server_request) = &mut actor.server_request {
            if server_request.status_response.is_some() {
                debug!("Returning because Actor is for a Status Check");
                return;
            }

            if let Some(server_conn) = &mut server_request.server_conn {
                let shutdown_flag = shutdown.clone();
                tokio::select! {
                    Some(msg) = actor.gateway_receiver.recv() => {
                        if let Err(e) = actor.handle_gateway_message(msg) {
                            warn!("Gateway handler error: {:?}", e);
                            shutdown_flag.store(true, Ordering::SeqCst);
                        }
                    }
                    Some(msg) = actor.server_receiver.recv() => {

                        if let MinecraftCommunication::Shutdown = msg {
                             debug!("Shutting down server (Received Shutdown message)");
                             server_conn.close().await.unwrap();
                             actor.server_receiver.close();
                             shutdown_flag.store(true, Ordering::SeqCst);
                         };

                        if let Err(e) = proxy_mode.handle_internal_server(msg, &mut actor).await {
                            warn!("Internal handler error: {:?}", e);
                            shutdown_flag.store(true, Ordering::SeqCst);
                        }
                    }

                    Ok(read_value) = server_conn.read() => {
                        if let Err(e) = proxy_mode.handle_external_server(read_value, &mut actor).await {
                            warn!("External handler error: {:?}", e);
                            shutdown_flag.store(true, Ordering::SeqCst);
                        }
                    }

                    else => {
                        debug!("All channels closed");
                        shutdown_flag.store(true, Ordering::SeqCst);
                    }
                }
            } else {
                warn!("Server connection is None L114");
            }
        } else {
            warn!("Server request is None");
        }
    }

    if actor.is_login
        && actor.server_request.is_some()
        && actor.server_request.as_ref().unwrap().server_conn.is_some()
    {

        //TODO: Update this system so server have a different player counter than the proxy global one
        TELEMETRY.update_player_count(
            -1,
            &actor
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
    let _ = client_sender.send(MinecraftCommunication::Shutdown).await;
    actor.server_receiver.close();
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
