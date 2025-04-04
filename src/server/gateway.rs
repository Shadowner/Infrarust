use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::{
    Mutex,
    mpsc::{self},
    oneshot,
};
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};

use super::{ServerRequest, ServerRequester, ServerResponse, backend::Server, cache::StatusCache};
use crate::core::config::service::ConfigurationService;
#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;
use crate::{
    Connection, FilterRegistry,
    cli::ShutdownController,
    core::{actors::supervisor::ActorSupervisor, config::ServerConfig, event::GatewayMessage},
    network::proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    protocol::minecraft::java::login::ServerBoundLoginStart,
    proxy_modes::ProxyModeEnum,
    security::BanSystemAdapter,
    with_filter_or,
};
use crate::{FilterError, network::packet::Packet};

pub struct Gateway {
    config_service: Arc<ConfigurationService>,
    status_cache: Arc<Mutex<StatusCache>>,
    _sender: mpsc::Sender<GatewayMessage>,
    pub actor_supervisor: Arc<ActorSupervisor>,
    shutdown_controller: Arc<ShutdownController>,
    filter_registry: Option<Arc<FilterRegistry>>,
}

impl Gateway {
    pub fn new(
        sender: mpsc::Sender<GatewayMessage>,
        config_service: Arc<ConfigurationService>,
        shutdown_controller: Arc<ShutdownController>,
        filter_registry: Option<Arc<FilterRegistry>>,
    ) -> Self {
        info!("Initializing ServerGateway");

        let gateway = Self {
            config_service,
            _sender: sender,
            actor_supervisor: Arc::new(ActorSupervisor::new()),
            status_cache: Arc::new(Mutex::new(StatusCache::new(Duration::from_secs(30)))),
            shutdown_controller,
            filter_registry,
        };

        // Start a background task for periodic health checks
        let supervisor = gateway.actor_supervisor.clone();
        let shutdown = gateway.shutdown_controller.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            let mut shutdown_rx = shutdown.subscribe().await;

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Health check task received shutdown signal");
                        break;
                    }
                    _ = interval.tick() => {
                        supervisor.health_check().await;
                    }
                }
            }
        });

        gateway
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<GatewayMessage>) {
        //TODO: For future use
        // Keep the gateway running until a shutdown message is received
        #[allow(clippy::never_loop)]
        while let Some(message) = receiver.recv().await {
            match message {
                GatewayMessage::Shutdown => {
                    debug!("Gateway received shutdown message");
                    break;
                }
            }
        }
        debug!("Gateway run loop exited");
    }

    pub async fn update_configurations(&self, configurations: Vec<ServerConfig>) {
        self.config_service
            .update_configurations(configurations)
            .await;
    }

    pub async fn remove_configuration(&self, config_id: &str) {
        self.config_service.remove_configuration(config_id).await;
    }

    async fn is_username_banned(&self, username: &str) -> Option<String> {
        if let Some(registry) = &self.filter_registry {
            let is_banned = matches!(
                with_filter_or!(
                    registry,
                    "global_ban_system",
                    BanSystemAdapter,
                    async |filter: &BanSystemAdapter| { filter.is_username_banned(username).await },
                    false
                ),
                Ok(true)
            );

            if is_banned {
                if let Ok(reason) = with_filter_or!(
                    registry,
                    "global_ban_system",
                    BanSystemAdapter,
                    async |filter: &BanSystemAdapter| {
                        filter.get_ban_reason_for_username(username).await
                    },
                    None
                ) {
                    return reason;
                }
            }
        }
        None
    }

    #[instrument(name = "client_connection_handling", skip(client, request, gateway), fields(
        domain = %request.domain,
        is_login = request.is_login,
        protocol_version = ?request.protocol_version,
        client_addr = %request.client_addr,
        session_id = %request.session_id
    ))]
    pub async fn handle_client_connection(
        mut client: Connection,
        request: ServerRequest,
        gateway: Arc<Gateway>,
    ) {
        let span = Span::current();
        debug!(
            "Starting client connection handling for domain: {}",
            request.domain
        );

        let username = if request.is_login {
            debug!("Processing login request");
            match Self::extract_username_from_request(&request) {
                Ok(name) => {
                    debug!("Parsed login packet for user: {}", name);

                    if let Some(reason) = gateway.is_username_banned(&name).await {
                        warn!(
                            "Player with banned username '{}' attempted to connect: {}",
                            name, reason
                        );
                        if let Err(e) = client.close().await {
                            warn!("Error closing connection for banned username: {:?}", e);
                        }
                        return;
                    }

                    name
                }
                Err(e) => {
                    warn!("Failed to parse login packet: {:?}", e);
                    if let Err(e) = client.close().await {
                        warn!("Error closing connection: {:?}", e);
                    }
                    return;
                }
            }
        } else {
            String::new()
        };

        debug!("Looking up server for domain: {}", request.domain);
        let server_config = match gateway
            .find_server(&request.domain)
            .instrument(span.clone())
            .await
        {
            Some(server) => {
                debug!("Found server config for domain: {}", request.domain);
                server
            }
            None => {
                warn!(
                    "Server not found for domain: '{}' requested by - {}",
                    request.domain, request.client_addr
                );
                if let Err(e) = client.close().await {
                    warn!("Error closing connection: {:?}", e);
                }
                return;
            }
        };

        let proxy_mode = gateway.determine_proxy_mode(&request, &server_config);
        let connecting_domain = request.domain.clone();

        debug!("Creating oneshot channel for server response");
        let (oneshot_request_sender, oneshot_request_receiver) = oneshot::channel();

        debug!("Creating actor pair");
        let actor_pair = gateway
            .actor_supervisor
            .create_actor_pair(
                &server_config.config_id,
                client,
                proxy_mode.clone(),
                oneshot_request_receiver,
                request.is_login,
                username.clone(),
                &connecting_domain,
            )
            .instrument(debug_span!(parent: span.clone(), "create_actors",
                username = %username,
                proxy_mode = ?proxy_mode
            ))
            .await;

        // For status requests, use a shorter timeout to prevent blocking
        let timeout_duration = if request.is_login {
            std::time::Duration::from_secs(30) // Longer timeout for login connections
        } else {
            std::time::Duration::from_secs(5) // Short timeout for status requests
        };

        let supervisor = gateway.actor_supervisor.clone();
        let server_config_clone = server_config.clone();

        debug!("Spawning task to wake up server");
        let is_login = request.is_login;

        let task_handle = tokio::spawn(
            async move {
                debug!("About to call wake_up_server");

                match tokio::time::timeout(
                    timeout_duration,
                    gateway.wake_up_server(request, server_config),
                )
                .await
                {
                    Ok(result) => match result {
                        Ok(response) => {
                            debug!("Successfully received server response");
                            if oneshot_request_sender.send(response).is_err() {
                                if is_login {
                                    warn!("Failed to send server response: receiver dropped");
                                    actor_pair
                                        .shutdown
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                } else {
                                    debug!("Oneshot channel closed, normal for status requests");
                                }
                            } else {
                                debug!("Successfully sent server response to channel");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to request server: {:?}", e);
                            if is_login {
                                actor_pair
                                    .shutdown
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        }
                    },
                    Err(_) => {
                        warn!("Timeout while waiting for server wake-up");
                        if is_login {
                            actor_pair
                                .shutdown
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                }

                debug!("Server wake-up task completed");
            }
            .instrument(span),
        );

        if is_login {
            info!(
                "Player '{}' connected to '{}' ({})",
                &username, connecting_domain, &server_config_clone.config_id
            );
        } else {
            debug!(
                "Status request for '{}' ({}) is being processed",
                connecting_domain, &server_config_clone.config_id
            );
        }

        debug!("Registering task with supervisor");
        supervisor
            .register_task(&server_config_clone.config_id, task_handle)
            .await;

        debug!("Client connection handling complete");
    }

    fn extract_username_from_request(request: &ServerRequest) -> Result<String, String> {
        let login_start = &request.read_packets[1];
        ServerBoundLoginStart::try_from(login_start)
            .map(|login| login.name.0.clone())
            .map_err(|e| format!("{:?}", e))
    }

    fn determine_proxy_mode(
        &self,
        request: &ServerRequest,
        server_config: &ServerConfig,
    ) -> ProxyModeEnum {
        if !request.is_login {
            debug!("Processing status request for domain: {}", request.domain);
            #[cfg(feature = "telemetry")]
            TELEMETRY.record_request();
            ProxyModeEnum::Status
        } else {
            debug!("Processing login request for domain: {}", request.domain);
            #[cfg(feature = "telemetry")]
            TELEMETRY.record_new_connection(
                &request.client_addr.to_string(),
                &request.domain,
                request.session_id,
            );
            server_config.proxy_mode.clone().unwrap_or_default()
        }
    }

    #[instrument(skip(self), fields(domain = %domain), level = "debug")]
    async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!("Finding server by domain: {}", domain);
        let configs = self.config_service.get_all_configurations().await;
        debug!("Got {} total server configurations", configs.len());

        let result = self.config_service.find_server_by_domain(domain).await;

        debug!(
            domain = %domain,
            found = result.is_some(),
            "Domain lookup result"
        );

        if result.is_some() {
            debug!("Found server for domain {}", domain);
        } else {
            debug!("No server found for domain {}", domain);
        }

        result
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        self.config_service.find_server_by_ip(ip).await
    }

    #[instrument(name = "wake_up_server_internal", skip(self, req, server), fields(
        domain = %req.domain,
        is_login = %req.is_login,
        server_addr = %server.addresses.first().unwrap_or(&String::new()),
        session_id = %req.session_id
    ))]
    async fn wake_up_server_internal(
        &self,
        req: ServerRequest,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        debug!("Creating server instance for {}", req.domain);
        let tmp_server = match Server::new(server.clone()) {
            Ok(s) => {
                debug!("Server instance created successfully");
                s
            }
            Err(e) => {
                error!("Failed to create server instance: {}", e);
                return self.generate_unreachable_motd_response(req.domain, server);
            }
        };

        // For status requests, use a fast-path that doesn't block on mutex acquisition
        if !req.is_login {
            return self.handle_status_request(&req, &tmp_server, server).await;
        }

        debug!("Creating login connection to backend server");
        self.handle_login_request(&req, &tmp_server, server).await
    }

    async fn handle_status_request(
        &self,
        req: &ServerRequest,
        tmp_server: &Server,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        debug!("Fast-path for status request to {}", req.domain);

        if let Some(response) = self.try_quick_cache_lookup(tmp_server, req).await {
            return self.create_status_response(req.domain.clone(), server, response, tmp_server);
        }

        debug!("No quick cache hit, fetching status directly from server");
        match tmp_server.fetch_status_directly(req).await {
            Ok(packet) => {
                // Update cache in the background without waiting
                self.update_cache_in_background(tmp_server, req, packet.clone());
                self.create_status_response(req.domain.clone(), server, packet, tmp_server)
            }
            Err(e) => {
                debug!("Status fetch failed: {}. Using unreachable MOTD", e);
                self.generate_unreachable_motd_response(req.domain.clone(), server)
            }
        }
    }

    async fn try_quick_cache_lookup(
        &self,
        tmp_server: &Server,
        req: &ServerRequest,
    ) -> Option<Packet> {
        match tokio::time::timeout(std::time::Duration::from_millis(100), async {
            let mut cache_guard = self.status_cache.lock().await;
            cache_guard.check_cache_only(tmp_server, req).await
        })
        .await
        {
            Ok(Ok(Some(response))) => {
                debug!("Got cached status response quickly");
                Some(response)
            }
            _ => None,
        }
    }

    fn update_cache_in_background(&self, tmp_server: &Server, req: &ServerRequest, packet: Packet) {
        let cache = self.status_cache.clone();
        let tmp_server_clone = tmp_server.clone();
        let req_clone = req.clone();
        let packet_clone = packet.clone();

        tokio::spawn(async move {
            if let Ok(mut cache_guard) = cache.try_lock() {
                cache_guard
                    .update_cache_for(&tmp_server_clone, &req_clone, packet_clone)
                    .await;
            }
        });
    }

    fn create_status_response(
        &self,
        domain: String,
        server: Arc<ServerConfig>,
        packet: Packet,
        tmp_server: &Server,
    ) -> ProtocolResult<ServerResponse> {
        Ok(ServerResponse {
            server_conn: None,
            status_response: Some(packet),
            send_proxy_protocol: tmp_server.config.send_proxy_protocol.unwrap_or_default(),
            read_packets: vec![],
            server_addr: None,
            proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(),
            proxied_domain: Some(domain),
            initial_config: server,
        })
    }

    async fn handle_login_request(
        &self,
        req: &ServerRequest,
        tmp_server: &Server,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let use_proxy_protocol = tmp_server.config.send_proxy_protocol.unwrap_or(false);
        let conn = if use_proxy_protocol {
            debug!("Using proxy protocol for connection");
            tmp_server
                .dial_with_proxy_protocol(req.session_id, req.client_addr)
                .await
        } else {
            debug!("Using standard connection");
            tmp_server.dial(req.session_id).await
        };

        match conn {
            Ok(connection) => {
                debug!("Login connection established successfully");
                Ok(ServerResponse {
                    server_conn: Some(connection),
                    status_response: None,
                    send_proxy_protocol: use_proxy_protocol,
                    read_packets: req.read_packets.to_vec(),
                    server_addr: Some(req.client_addr),
                    proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(),
                    proxied_domain: Some(req.domain.clone()),
                    initial_config: server,
                })
            }
            Err(e) => {
                debug!("Failed to connect to backend server: {}", e);
                Err(e)
            }
        }
    }

    fn generate_unreachable_motd_response(
        &self,
        domain: String,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        debug!("Generating unreachable MOTD response for {}", domain);

        let guard = crate::CONFIG.read();

        let motd_packet = if let Some(motd) = &server.motd {
            debug!("Using server-specific MOTD for unreachable status");
            crate::server::motd::generate_motd(motd, false)?
        } else if let Some(motd) = guard.motds.unreachable.clone() {
            debug!("Using global unreachable MOTD");
            crate::server::motd::generate_motd(&motd, true)?
        } else {
            debug!("Using default unreachable MOTD");
            crate::server::motd::generate_motd(
                &crate::server::motd::MotdConfig::default_unreachable(),
                true,
            )?
        };

        Ok(ServerResponse {
            server_conn: None,
            status_response: Some(motd_packet),
            send_proxy_protocol: false,
            read_packets: vec![],
            server_addr: None,
            proxy_mode: ProxyModeEnum::Status,
            proxied_domain: Some(domain),
            initial_config: server,
        })
    }

    async fn handle_unknown_server(&self, req: &ServerRequest) -> ProtocolResult<ServerResponse> {
        let guard = crate::CONFIG.read();

        let fake_config = Arc::new(ServerConfig {
            domains: vec![req.domain.clone()],
            addresses: vec![],
            config_id: format!("unknown_{}", req.domain),
            ..ServerConfig::default()
        });

        if let Some(motd) = guard.motds.unknown.clone() {
            debug!("Generating unknown server MOTD for {}", req.domain);
            let motd_packet = crate::server::motd::generate_motd(&motd, true)?;

            return Ok(ServerResponse {
                server_conn: None,
                status_response: Some(motd_packet),
                send_proxy_protocol: false,
                read_packets: vec![],
                server_addr: None,
                proxy_mode: ProxyModeEnum::Status,
                proxied_domain: Some(req.domain.clone()),
                initial_config: fake_config,
            });
        }

        Err(ProxyProtocolError::Other(format!(
            "Server not found for domain: {}",
            req.domain
        )))
    }
}

#[async_trait]
impl ServerRequester for Gateway {
    #[instrument(name = "request_server", skip(self, req), fields(
        domain = %req.domain,
        is_login = req.is_login,
        session_id = %req.session_id
    ))]
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse> {
        debug!("Requesting server for domain: {}", req.domain);
        let server_config = match self
            .find_server(&req.domain)
            .instrument(debug_span!("server_request: find_server"))
            .await
        {
            Some(config) => {
                debug!("Found server for domain: {}", req.domain);
                config
            }
            None => {
                debug!(
                    "Server not found for domain: {}, using unreachable MOTD",
                    req.domain
                );

                if req.is_login {
                    return Err(ProxyProtocolError::Other(format!(
                        "Server not found for domain: {}",
                        req.domain
                    )));
                }

                return self.handle_unknown_server(&req).await;
            }
        };

        debug!(
            "Found server for domain: {}, proceeding to wake up",
            req.domain
        );
        self.wake_up_server_internal(req, server_config)
            .instrument(debug_span!("server_request: wake_up_server"))
            .await
    }

    async fn wake_up_server(
        &self,
        req: ServerRequest,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let domain_ref = &req.domain.clone();
        debug!("Wake up server: {} with {}", domain_ref, &server.config_id);
        let result = self.wake_up_server_internal(req, server).await;
        match &result {
            Ok(_) => debug!("Wake up server successful for: {}", domain_ref),
            Err(e) => debug!("Wake up server failed for: {}: {}", domain_ref, e),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
    fn setup_test_server() -> (TcpListener, String) {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr.to_string())
    }

    #[test]
    fn test_server_gateway() {
        let (_listener, _addr) = setup_test_server();
        // let server_config = ServerConfig {
        //     domains: vec!["example.com".to_string()],
        //     addresses: vec![addr],
        //     send_proxy_protocol: Some(false),
        //     proxy_mode: Some(ProxyModeEnum::Passthrough),
        // };

        // let gateway = Gateway::new(vec![server_config]);

        // assert!(gateway.find_server("example.com").is_some());
        // assert!(gateway.find_server("other.com").is_none());
        // TODO: Add more comprehensive tests for status caching and request handling
    }
}
