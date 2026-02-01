use std::sync::{Arc, atomic::AtomicBool};

use infrarust_config::{LogType, models::server::ProxyModeEnum};
use tokio::sync::mpsc;
use tracing::{Instrument, debug, debug_span, info, instrument, warn};

use crate::{
    Connection,
    core::{
        actors::{
            client::{ClientHandlerConfig, MinecraftClientHandler},
            server::MinecraftServerHandler,
        },
        event::MinecraftCommunication,
    },
    proxy_modes::{
        ClientProxyModeHandler, ProxyMessage, ServerProxyModeHandler,
        client_only::rewrite_handshake_domain, spawn_splice_task,
    },
    server::ServerResponse,
};

use super::ActorSupervisor;

/// A pair of client and server actors for handling a Minecraft connection
#[derive(Clone, Debug)]
pub struct ActorPair {
    pub username: String,
    pub client: MinecraftClientHandler,
    pub server: MinecraftServerHandler,
    pub shutdown: Arc<AtomicBool>,
    pub created_at: std::time::Instant,
    pub session_id: uuid::Uuid,
    pub config_id: String,
    pub server_name: String,
    pub disconnect_logged: Arc<AtomicBool>,
    pub is_login: bool,
    pub zerocopy_task: Option<Arc<tokio::task::JoinHandle<(u64, u64)>>>,
}

impl ActorSupervisor {
    #[allow(clippy::too_many_arguments)] // Use builder() for cleaner API
    #[instrument(name = "supervisor_create_pair", skip(self, client_conn, proxy_mode, oneshot_request_receiver), fields(
        config_id = %config_id,
        username = %username,
        proxy_mode = ?proxy_mode,
        is_login = is_login
    ))]
    pub async fn create_actor_pair(
        &self,
        config_id: &str,
        client_conn: Connection,
        proxy_mode: ProxyModeEnum,
        oneshot_request_receiver: tokio::sync::oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        domain: &str,
    ) -> ActorPair {
        use crate::proxy_modes::{
            get_client_only_mode, get_offline_mode, get_passthrough_mode, get_status_mode,
        };

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let span = debug_span!("actor_pair_setup", log_type = LogType::Supervisor.as_str());
        let session_id = client_conn.session_id;

        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Creating actor pair with session_id: {}, is_login: {}, proxy_mode: {:?}",
            session_id,
            is_login,
            proxy_mode
        );

        if is_login {
            #[cfg(feature = "telemetry")]
            crate::telemetry::TELEMETRY.update_player_count(
                1,
                config_id,
                client_conn.session_id,
                &username,
            );
        }

        // Macro to reduce boilerplate for creating actor pairs with different handler types
        macro_rules! create_pair_with_mode {
            ($get_mode:expr) => {{
                let (client_handler, server_handler) = $get_mode;
                self.create_actor_pair_with_handlers(
                    config_id,
                    client_conn,
                    client_handler,
                    server_handler,
                    oneshot_request_receiver,
                    is_login,
                    username,
                    shutdown_flag,
                    session_id,
                    domain.to_string(),
                )
                .instrument(span)
                .await
            }};
        }

        if proxy_mode == ProxyModeEnum::ZeroCopyPassthrough {
            if let Some(pair) = self
                .create_zerocopy_actor_pair(
                    config_id,
                    client_conn,
                    oneshot_request_receiver,
                    is_login,
                    username.clone(),
                    domain,
                )
                .instrument(span)
                .await
            {
                self.register_actor_pair(config_id, pair.clone())
                    .instrument(debug_span!("register_pair"))
                    .await;

                debug!(
                    log_type = LogType::Supervisor.as_str(),
                    "Zerocopy actor pair created successfully"
                );
                return pair;
            } else {
                warn!(
                    log_type = LogType::Supervisor.as_str(),
                    "Zerocopy actor pair creation failed, connection lost"
                );
                let client = MinecraftClientHandler::new_zerocopy_stub(
                    std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
                        std::net::Ipv4Addr::new(0, 0, 0, 0),
                        0,
                    )),
                    shutdown_flag.clone(),
                    session_id,
                );
                let server = MinecraftServerHandler::new_zerocopy_stub(shutdown_flag.clone());
                shutdown_flag.store(true, std::sync::atomic::Ordering::SeqCst);

                return ActorPair {
                    username,
                    client,
                    server,
                    shutdown: shutdown_flag,
                    created_at: std::time::Instant::now(),
                    session_id,
                    config_id: config_id.to_string(),
                    server_name: domain.to_string(),
                    disconnect_logged: Arc::new(AtomicBool::new(false)),
                    is_login,
                    zerocopy_task: None,
                };
            }
        }

        let pair = match proxy_mode {
            ProxyModeEnum::Status => create_pair_with_mode!(get_status_mode()),
            ProxyModeEnum::Passthrough | ProxyModeEnum::ServerOnly => {
                create_pair_with_mode!(get_passthrough_mode())
            }
            ProxyModeEnum::Offline => create_pair_with_mode!(get_offline_mode()),
            ProxyModeEnum::ClientOnly => create_pair_with_mode!(get_client_only_mode()),
            ProxyModeEnum::ZeroCopyPassthrough => {
                unreachable!("ZeroCopyPassthrough handled above")
            }
        };

        self.register_actor_pair(config_id, pair.clone())
            .instrument(debug_span!("register_pair"))
            .await;

        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Actor pair created successfully"
        );
        pair
    }

    #[instrument(skip(self, client_conn, client_handler, server_handler, oneshot_request_receiver, shutdown_flag), fields(
        config_id = %config_id,
        username = %username,
        is_login = is_login
    ))]
    #[allow(clippy::too_many_arguments)] // Internal generic method - use builder() for public API
    pub(crate) async fn create_actor_pair_with_handlers<T>(
        &self,
        config_id: &str,
        client_conn: Connection,
        client_handler: Box<dyn ClientProxyModeHandler<MinecraftCommunication<T>>>,
        server_handler: Box<dyn ServerProxyModeHandler<MinecraftCommunication<T>>>,
        oneshot_request_receiver: tokio::sync::oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        shutdown_flag: Arc<AtomicBool>,
        session_id: uuid::Uuid,
        server_name: String,
    ) -> ActorPair
    where
        T: ProxyMessage + 'static + Send + Sync + std::fmt::Debug,
    {
        let (server_sender, server_receiver) = mpsc::channel(64);
        let (client_sender, client_receiver) = mpsc::channel(64);

        let root_span = if is_login {
            Some(debug_span!(
                parent: None,
                "actor_handling",
                username = %username,
                is_login = is_login
            ))
        } else {
            None
        };

        let client = MinecraftClientHandler::new(ClientHandlerConfig {
            server_sender,
            client_receiver,
            proxy_mode: client_handler,
            conn: client_conn,
            is_login,
            username: username.clone(),
            shutdown: shutdown_flag.clone(),
            start_span: root_span.clone(),
        })
        .await;

        let server = MinecraftServerHandler::new(
            client_sender,
            server_receiver,
            is_login,
            oneshot_request_receiver,
            server_handler,
            shutdown_flag.clone(),
            root_span.clone(),
        );

        ActorPair {
            username: username.clone(),
            client,
            server,
            shutdown: shutdown_flag,
            created_at: std::time::Instant::now(),
            session_id,
            config_id: config_id.to_string(),
            server_name,
            disconnect_logged: Arc::new(AtomicBool::new(false)),
            is_login,
            zerocopy_task: None,
        }
    }

    /// This mode bypasses the normal actor message passing for data transfer,
    /// using splice() (on Linux) or optimized userspace copy for direct
    /// TCP-to-TCP data flow after the initial handshake.
    #[allow(clippy::too_many_arguments)]
    #[instrument(name = "supervisor_create_zerocopy_pair", skip(self, client_conn, oneshot_request_receiver), fields(
        config_id = %config_id,
        username = %username,
        is_login = is_login
    ))]
    pub(crate) async fn create_zerocopy_actor_pair(
        &self,
        config_id: &str,
        client_conn: Connection,
        oneshot_request_receiver: tokio::sync::oneshot::Receiver<ServerResponse>,
        is_login: bool,
        username: String,
        domain: &str,
    ) -> Option<ActorPair> {
        use std::sync::atomic::Ordering::SeqCst;

        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let session_id = client_conn.session_id;

        info!(
            log_type = LogType::Supervisor.as_str(),
            "Creating zerocopy actor pair with session_id: {}", session_id
        );

        let client_peer_addr = match client_conn.peer_addr().await {
            Ok(addr) => addr,
            Err(e) => {
                warn!(
                    log_type = LogType::Supervisor.as_str(),
                    "Failed to get client peer address: {:?}", e
                );
                return None;
            }
        };

        let client = MinecraftClientHandler::new_zerocopy_stub(
            client_peer_addr,
            shutdown_flag.clone(),
            session_id,
        );
        let server = MinecraftServerHandler::new_zerocopy_stub(shutdown_flag.clone());

        let server_name = domain.to_string();
        let shutdown_for_task = shutdown_flag.clone();
        tokio::spawn(async move {
            let server_response = match oneshot_request_receiver.await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!(
                        log_type = LogType::Supervisor.as_str(),
                        "Failed to receive server response for zerocopy: {:?}", e
                    );
                    shutdown_for_task.store(true, SeqCst);
                    return;
                }
            };
            let mut server_conn = match server_response.server_conn {
                Some(conn) => conn,
                None => {
                    warn!(
                        log_type = LogType::Supervisor.as_str(),
                        "No server connection in response for zerocopy mode"
                    );
                    shutdown_for_task.store(true, SeqCst);
                    return;
                }
            };
            let effective_domain = server_response
                .initial_config
                .get_effective_backend_domain();

            debug!(
                log_type = LogType::ProxyMode.as_str(),
                "Zerocopy domain rewrite config - backend_domain: {:?}, rewrite_domain: {}, effective: {:?}",
                server_response.initial_config.backend_domain,
                server_response.initial_config.rewrite_domain,
                effective_domain
            );

            for (i, packet) in server_response.read_packets.iter().enumerate() {
                if i == 0
                    && let Some(ref new_domain) = effective_domain
                {
                    debug!(
                        log_type = LogType::ProxyMode.as_str(),
                        "Rewriting handshake domain to: {}", new_domain
                    );
                    match rewrite_handshake_domain(packet, new_domain) {
                        Ok(rewritten_packet) => {
                            if let Err(e) = server_conn.write_packet(&rewritten_packet).await {
                                warn!("Failed to send rewritten handshake packet: {:?}", e);
                                shutdown_for_task.store(true, SeqCst);
                                return;
                            }
                            continue;
                        }
                        Err(e) => {
                            warn!("Failed to rewrite handshake domain: {:?}", e);
                            shutdown_for_task.store(true, SeqCst);
                            return;
                        }
                    }
                }
                if let Err(e) = server_conn.write_packet(packet).await {
                    warn!("Failed to send handshake packet: {:?}", e);
                    shutdown_for_task.store(true, SeqCst);
                    return;
                }
            }

            if let Err(e) = server_conn.flush().await {
                warn!("Failed to flush server connection: {:?}", e);
                shutdown_for_task.store(true, SeqCst);
                return;
            }

            let client_stream = match client_conn.into_tcp_stream_async().await {
                Ok(stream) => stream,
                Err(e) => {
                    warn!(
                        log_type = LogType::Supervisor.as_str(),
                        "Failed to extract client TcpStream: {:?}", e
                    );
                    shutdown_for_task.store(true, SeqCst);
                    return;
                }
            };

            let server_stream = match server_conn.into_tcp_stream_async().await {
                Ok(stream) => stream,
                Err(e) => {
                    warn!(
                        log_type = LogType::Supervisor.as_str(),
                        "Failed to extract server TcpStream: {:?}", e
                    );
                    shutdown_for_task.store(true, SeqCst);
                    return;
                }
            };

            info!(
                log_type = LogType::Supervisor.as_str(),
                "Zerocopy splice setup complete, starting data transfer"
            );

            let splice_task =
                spawn_splice_task(client_stream, server_stream, shutdown_for_task.clone());
            match splice_task.await {
                Ok((client_to_server, server_to_client)) => {
                    debug!(
                        log_type = LogType::Supervisor.as_str(),
                        "Zerocopy splice completed: {} bytes client->server, {} bytes server->client",
                        client_to_server,
                        server_to_client
                    );
                }
                Err(e) => {
                    warn!(
                        log_type = LogType::Supervisor.as_str(),
                        "Zerocopy splice task error: {:?}", e
                    );
                }
            }

            shutdown_for_task.store(true, SeqCst);
        });

        Some(ActorPair {
            username,
            client,
            server,
            shutdown: shutdown_flag,
            created_at: std::time::Instant::now(),
            session_id,
            config_id: config_id.to_string(),
            server_name,
            disconnect_logged: Arc::new(AtomicBool::new(false)),
            is_login,
            zerocopy_task: None, // Task is managed internally by the background spawn
        })
    }

    pub async fn register_legacy_actor_pair(
        &self,
        config_id: &str,
        username: String,
        server_name: String,
        client_addr: std::net::SocketAddr,
        session_id: uuid::Uuid,
    ) -> Arc<AtomicBool> {
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let client = MinecraftClientHandler::new_zerocopy_stub(
            client_addr,
            shutdown_flag.clone(),
            session_id,
        );
        let server = MinecraftServerHandler::new_zerocopy_stub(shutdown_flag.clone());

        let pair = ActorPair {
            username,
            client,
            server,
            shutdown: shutdown_flag.clone(),
            created_at: std::time::Instant::now(),
            session_id,
            config_id: config_id.to_string(),
            server_name,
            disconnect_logged: Arc::new(AtomicBool::new(false)),
            is_login: true,
            zerocopy_task: None,
        };

        self.register_actor_pair(config_id, pair).await;

        shutdown_flag
    }

    #[instrument(skip(self, pair), fields(config_id = %config_id))]
    pub(crate) async fn register_actor_pair(&self, config_id: &str, pair: ActorPair) {
        let mut actors = self.actors.write().await;
        actors
            .entry(config_id.to_string())
            .or_insert_with(Vec::new)
            .push(pair);
    }
}
