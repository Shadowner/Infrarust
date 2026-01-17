use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use infrarust_config::{
    ServerConfig,
    models::{logging::LogType, server::ProxyModeEnum},
};
use infrarust_protocol::minecraft::java::login::ServerBoundLoginStart;
use infrarust_server_manager::ServerState;
use tokio::sync::{
    Mutex,
    mpsc::{self},
    oneshot,
    watch::{self, Receiver},
};
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};

use super::{ServerRequest, ServerRequester, ServerResponse, backend::Server, cache::StatusCache};
#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;
use crate::{
    Connection,
    core::event::GatewayMessage,
    network::proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    server::motd::{self, MotdState, generate_response},
};
use crate::{core::shared_component::SharedComponent, network::connection::PossibleReadValue};
use crate::network::packet::Packet;
use crate::security::BanHelper;

static SHARED_COMPONENT: std::sync::OnceLock<Arc<SharedComponent>> = std::sync::OnceLock::new();
#[derive(Debug, Clone)]
pub struct Gateway {
    status_cache: Arc<Mutex<StatusCache>>,
    shared: Arc<SharedComponent>,
    #[allow(clippy::type_complexity)]
    pending_status_requests:
        Arc<Mutex<HashMap<u64, Receiver<Option<Result<Packet, ProxyProtocolError>>>>>>,
}

impl Gateway {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        info!(
            log_type = LogType::Authentication.as_str(),
            "Initializing ServerGateway"
        );

        let _ = SHARED_COMPONENT.set(shared.clone());

        let config = shared.config();
        let gateway = Self {
            status_cache: Arc::new(Mutex::new(StatusCache::from_shared_config(config))),
            pending_status_requests: Arc::new(Mutex::new(HashMap::new())),
            shared,
        };

        // Start a background task for periodic health checks
        let supervisor = gateway.shared.actor_supervisor();
        let shutdown = gateway.shared.shutdown_controller();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            let mut shutdown_rx = shutdown.subscribe().await;

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!(log_type = LogType::Authentication.as_str(), "Health check task received shutdown signal");
                        break;
                    }
                    _ = interval.tick() => {
                        supervisor.health_check().await;
                        supervisor.check_and_mark_empty_servers().await;
                    }
                }
            }
        });

        gateway
    }

    pub fn get_shared_component() -> Option<Arc<SharedComponent>> {
        SHARED_COMPONENT.get().cloned()
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<GatewayMessage>) {
        //TODO: For future use
        // Keep the gateway running until a shutdown message is received
        #[allow(clippy::never_loop)]
        while let Some(message) = receiver.recv().await {
            match message {
                GatewayMessage::Shutdown => {
                    debug!(
                        log_type = LogType::Authentication.as_str(),
                        "Gateway received shutdown message"
                    );
                    break;
                }
            }
        }
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Gateway run loop exited"
        );
    }

    pub async fn update_configurations(&self, configurations: Vec<ServerConfig>) {
        self.shared
            .configuration_service()
            .update_configurations(configurations)
            .await;
    }

    pub async fn remove_configuration(&self, config_id: &str) {
        self.shared
            .configuration_service()
            .remove_configuration(config_id)
            .await;
    }

    async fn is_username_banned(&self, username: &str) -> Option<String> {
        BanHelper::is_username_banned(&self.shared.filter_registry(), username).await
    }

    #[instrument(name = "client_connection_handling", skip(client, request), fields(
        domain = %request.domain,
        is_login = request.is_login,
        protocol_version = ?request.protocol_version,
        client_addr = %request.client_addr,
        session_id = %request.session_id
    ))]
    pub async fn handle_client_connection(&self, mut client: Connection, request: ServerRequest) {
        let span = Span::current();
        debug!(
            "Starting client connection handling for domain: {}",
            request.domain
        );

        let username = if request.is_login {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Processing login request"
            );
            match Self::extract_username_from_request(&request) {
                Ok(name) => {
                    debug!(
                        log_type = LogType::Authentication.as_str(),
                        "Parsed login packet for user: {}", name
                    );

                    if let Some(reason) = self.is_username_banned(&name).await {
                        warn!(
                            log_type = "ban_system",
                            "Player with banned username '{}' attempted to connect: {}",
                            name,
                            reason
                        );
                        if let Err(e) = client.close().await {
                            warn!(
                                log_type = LogType::TcpConnection.as_str(),
                                "Error closing connection for banned username: {:?}", e
                            );
                        }
                        return;
                    }

                    name
                }
                Err(e) => {
                    warn!(
                        log_type = LogType::TcpConnection.as_str(),
                        "Failed to parse login packet: {:?}", e
                    );
                    if let Err(e) = client.close().await {
                        warn!(
                            log_type = LogType::TcpConnection.as_str(),
                            "Error closing connection: {:?}", e
                        );
                    }
                    return;
                }
            }
        } else {
            String::new()
        };

        debug!(
            log_type = LogType::TcpConnection.as_str(),
            "Looseking up server for domain: {}", request.domain
        );
        let server_config = {
            let domain = request.domain.clone();
            self.find_server(&domain).await
        };
        let server_config = match server_config {
            Some(config) => config,
            None => return,
        };

        let proxy_mode = self.determine_proxy_mode(&request, &server_config);

        if proxy_mode == ProxyModeEnum::Status {
            debug!(
                log_type = LogType::TcpConnection.as_str(),
                "Handling status request directly without creating actors"
            );
            self.handle_status_request_directly(client, request, server_config)
                .await;
            return;
        }

        if let Some(manager_config) = &server_config.server_manager {
            debug!(
                log_type = LogType::ServerManager.as_str(),
                "Server manager is present, checking status"
            );
            let server_manager = self
                .shared
                .server_managers()
                .get_status_for_server(&manager_config.server_id, manager_config.provider_name)
                .await;

            if let Ok(manager) = server_manager {
                let server_id = manager_config.server_id.clone();
                let manager_type = manager_config.provider_name;

                if manager.state == ServerState::Crashed {
                    warn!(
                        log_type = LogType::ServerManager.as_str(),
                        "Server {} is crashed, using unreachable MOTD", server_config.config_id
                    );
                }

                if manager.state == ServerState::Stopped {
                    warn!(
                        log_type = LogType::ServerManager.as_str(),
                        "Trying to start Server {}", server_config.config_id
                    );
                    let start_server = self
                        .shared
                        .server_managers()
                        .start_server(&server_id, manager_type)
                        .await;

                    if let Err(e) = start_server {
                        warn!(
                            log_type = LogType::ServerManager.as_str(),
                            "Failed to start server {}: {:?}", server_config.config_id, e
                        );
                    }
                }

                if manager.state != ServerState::Running {
                    if let Err(e) = client.close().await {
                        warn!(
                            log_type = LogType::TcpConnection.as_str(),
                            "Error closing connection: {:?}", e
                        );
                    }
                    return;
                }

                if manager.state == ServerState::Running {
                    let _ = self
                        .shared
                        .server_managers()
                        .remove_server_from_empty(&server_id, manager_type)
                        .await;
                }
            }
        }

        let connecting_domain = request.domain.clone();

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Creating oneshot channel for server response"
        );
        let (oneshot_request_sender, oneshot_request_receiver) = oneshot::channel();

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Creating actor pair"
        );
        let actor_pair = self
            .shared
            .actor_supervisor()
            .create_actor_pair(
                &server_config.config_id,
                client,
                proxy_mode,
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

        let supervisor = self.shared.actor_supervisor().clone();
        let server_config_clone = server_config.clone();

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Spawning task to wake up server"
        );
        let is_login = request.is_login;

        let self_guard = self.clone();
        let task_handle = tokio::spawn(
            async move {
                debug!(
                    log_type = LogType::Authentication.as_str(),
                    "About to call wake_up_server"
                );

                match tokio::time::timeout(
                    timeout_duration,
                    self_guard.wake_up_server(request, server_config),
                )
                .await
                {
                    Ok(result) => match result {
                        Ok(response) => {
                            debug!(
                                log_type = LogType::ServerManager.as_str(),
                                "Successfully received server response"
                            );
                            if oneshot_request_sender.send(response).is_err() {
                                if is_login {
                                    warn!(
                                        log_type = LogType::ServerManager.as_str(),
                                        "Failed to send server response: receiver dropped"
                                    );
                                    actor_pair
                                        .shutdown
                                        .store(true, std::sync::atomic::Ordering::SeqCst);
                                } else {
                                    debug!(
                                        log_type = LogType::ServerManager.as_str(),
                                        "Oneshot channel closed, normal for status requests"
                                    );
                                }
                            } else {
                                debug!(
                                    log_type = LogType::Authentication.as_str(),
                                    "Successfully sent server response to channel"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                log_type = LogType::Authentication.as_str(),
                                "Failed to request server: {:?}", e
                            );
                            if is_login {
                                actor_pair
                                    .shutdown
                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        }
                    },
                    Err(_) => {
                        warn!(
                            log_type = LogType::Authentication.as_str(),
                            "Timeout while waiting for server wake-up"
                        );
                        if is_login {
                            actor_pair
                                .shutdown
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    }
                }

                debug!(
                    log_type = LogType::Authentication.as_str(),
                    "Server wake-up task completed"
                );
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

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Registering task with supervisor"
        );
        supervisor
            .register_task(&server_config_clone.config_id, task_handle)
            .await;

        debug!(
            log_type = LogType::Authentication.as_str(),
            "Client connection handling complete"
        );
    }
    #[instrument(name = "handle_status_request_directly", skip(self, client, request), fields(
        domain = %request.domain,
        client_addr = %request.client_addr,
        session_id = %request.session_id
    ))]
    pub async fn handle_status_request_directly(
        &self,
        mut client: Connection,
        request: ServerRequest,
        server_config: Arc<ServerConfig>,
    ) {
        debug!(
            "Handling status request directly for domain: {}",
            request.domain
        );

        let self_clone = self.clone();
        tokio::spawn(async move {
            let near_shutdown_threshold = 60; // Increased to 300 seconds (5 minutes) to show shutdown MOTD earlier

            let response: Result<ServerResponse, ProxyProtocolError> = match &server_config
                .server_manager
            {
                Some(config) => {
                    // Check if this server is near shutdown
                    let server_managers = self_clone.shared.server_managers();
                    let near_shutdown_servers = server_managers
                        .get_servers_near_shutdown(near_shutdown_threshold)
                        .await;

                    // Check if this specific server is in the near-shutdown list
                    let mut is_near_shutdown = false;
                    let mut remaining_seconds = 0;

                    for (server_id, manager_type, seconds) in near_shutdown_servers {
                        if server_id == config.server_id && manager_type == config.provider_name {
                            is_near_shutdown = true;
                            remaining_seconds = seconds;
                            break;
                        }
                    }

                    if is_near_shutdown {
                        debug!(
                            "Server {} is scheduled to shut down in {} seconds",
                            server_config.config_id, remaining_seconds
                        );
                        generate_response(
                            MotdState::ImminentShutdown { seconds_remaining: remaining_seconds },
                            request.domain,
                            server_config.clone(),
                        )
                    } else {
                        let status = self_clone
                            .shared
                            .server_managers()
                            .get_status_for_server(&config.server_id, config.provider_name)
                            .await;

                        match status {
                            Err(e) => {
                                error!(
                                    "Failed to get status for server {} from manager {:?}: {}",
                                    config.server_id, config.provider_name, e
                                );
                                generate_response(MotdState::UnableToFetchStatus, request.domain, server_config)
                            }
                            Ok(server_status) => match server_status.state {
                                ServerState::Crashed => {
                                    warn!(
                                        "Server {} is crashed, using unreachable MOTD",
                                        server_config.config_id
                                    );
                                    generate_response(MotdState::Crashed, request.domain, server_config)
                                }
                                ServerState::Running => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is running", server_config.config_id
                                    );
                                    self_clone
                                        .get_or_fetch_status_response(
                                            request.clone(),
                                            server_config,
                                        )
                                        .await
                                }
                                ServerState::Starting => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is starting", server_config.config_id
                                    );
                                    generate_response(MotdState::Starting, request.domain, server_config)
                                }
                                ServerState::Stopped => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is stopped", server_config.config_id
                                    );
                                    generate_response(
                                        MotdState::Offline,
                                        request.domain,
                                        server_config,
                                    )
                                }
                                ServerState::Unknown => {
                                    error!(
                                        "Server {} is in unknown state",
                                        server_config.config_id
                                    );
                                    generate_response(MotdState::Crashed, request.domain, server_config)
                                }
                                ServerState::Stopping => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is stopping", server_config.config_id
                                    );
                                    generate_response(MotdState::Stopping, request.domain, server_config)
                                }
                            },
                        }
                    }
                }
                None => {
                    self_clone
                        .get_or_fetch_status_response(request.clone(), server_config)
                        .await
                }
            };

            match response {
                Ok(response) => {
                    if let Some(status_packet) = response.status_response {
                        debug!(
                            log_type = LogType::Authentication.as_str(),
                            "Sending status packet directly to client"
                        );
                        if let Err(e) = client.write_packet(&status_packet).await {
                            warn!(
                                log_type = LogType::Authentication.as_str(),
                                "Failed to send status packet to client: {:?}", e
                            );
                        }

                        // Wait briefly for potential ping packet
                        match tokio::time::timeout(
                            tokio::time::Duration::from_secs(2),
                            client.read(),
                        )
                        .await
                        {
                            Ok(Ok(PossibleReadValue::Packet(ping_packet))) => {
                                // If we got a ping packet, echo it back
                                debug!(
                                    log_type = LogType::Authentication.as_str(),
                                    "Received ping packet, echoing back"
                                );
                                if let Err(e) = client.write_packet(&ping_packet).await {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Failed to send ping response: {:?}", e
                                    );
                                }
                            }
                            _ => {
                                debug!(
                                    log_type = LogType::Authentication.as_str(),
                                    "No ping packet received or connection closed"
                                );
                            }
                        }
                    } else {
                        warn!(
                            log_type = LogType::Authentication.as_str(),
                            "No status response available for the request"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        log_type = LogType::Authentication.as_str(),
                        "Failed to get status response: {:?}", e
                    );
                }
            };

            // Always close the connection when done
            if let Err(e) = client.close().await {
                warn!(
                    log_type = LogType::Authentication.as_str(),
                    "Error closing connection after status response: {:?}", e
                );
            }
        });
    }

    // New method to get or fetch status response with request coalescing
    async fn get_or_fetch_status_response(
        &self,
        req: ServerRequest,
        server_config: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let tmp_server = match Server::new(server_config.clone()) {
            Ok(s) => s,
            Err(e) => {
                error!(
                    log_type = LogType::Authentication.as_str(),
                    "Failed to create server instance: {}", e
                );
                return self.generate_unreachable_motd_response(req.domain, server_config);
            }
        };

        // Use consistent key for request deduplication
        let cache = self.status_cache.lock().await;
        let key = cache.cache_key(&tmp_server, req.protocol_version);
        drop(cache);

        // Check if there's already a cached entry
        if let Some(packet) = self.try_quick_cache_lookup(&tmp_server, &req).await {
            return self.create_status_response(
                req.domain.clone(),
                server_config,
                packet,
                &tmp_server,
            );
        }

        if server_config.motds.online.is_some() {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Using online MOTD for {}", req.domain
            );
            return generate_response(MotdState::Online, req.domain, server_config);
        }

        // Check for pending requests - if one exists, wait for it instead of making a new request
        let pending_receiver = {
            let mut pending_requests = self.pending_status_requests.lock().await;

            if let Some(receiver) = pending_requests.get(&key).cloned() {
                // Another request is already in progress, wait for it
                debug!(
                    "Waiting for in-progress status request for {} (key: {})",
                    req.domain, key
                );
                Some(receiver)
            } else {
                // No pending request, create a new sender/receiver pair
                let (sender, receiver) = watch::channel(None);
                pending_requests.insert(key, receiver.clone());

                // Spawn a task to fetch the status
                let self_clone = self.clone();
                let tmp_server_clone = tmp_server.clone();
                let req_clone = req.clone();
                let server_config_clone = server_config.clone();

                tokio::spawn(async move {
                    let result = match tmp_server_clone.fetch_status_directly(&req_clone).await {
                        Ok(packet) => {
                            // Update cache
                            let mut cache = self_clone.status_cache.lock().await;
                            cache
                                .update_cache_for(&tmp_server_clone, &req_clone, packet.clone())
                                .await;

                            Ok(packet)
                        }
                        Err(e) => {
                            debug!(
                                log_type = LogType::Authentication.as_str(),
                                "Status fetch failed: {}. Using unreachable MOTD", e
                            );
                            // Get the error MOTD packet
                            let motd_response = self_clone.generate_unreachable_motd_response(
                                req_clone.domain.clone(),
                                server_config_clone,
                            );

                            match motd_response {
                                Ok(resp) => {
                                    if let Some(packet) = resp.status_response {
                                        Ok(packet)
                                    } else {
                                        Err(e)
                                    }
                                }
                                Err(_) => Err(e),
                            }
                        }
                    };

                    // Send the result to all waiters
                    let _ = sender.send(Some(result));

                    // Clean up the pending request
                    let mut pending_requests = self_clone.pending_status_requests.lock().await;
                    pending_requests.remove(&key);
                });

                Some(receiver)
            }
        };

        // Wait for the pending request to complete
        if let Some(mut receiver) = pending_receiver {
            // Wait for the result to be available
            let mut result = None;
            while result.is_none() {
                let _ = receiver.changed().await;
                result = receiver.borrow().clone();
            }

            // Result is guaranteed to be Some due to the while loop above
            match result.expect("Result should be Some after while loop") {
                Ok(packet) => {
                    self.create_status_response(req.domain, server_config, packet, &tmp_server)
                }
                Err(e) => Err(e),
            }
        } else {
            // This should never happen, but if it does, fall back to the original implementation
            debug!(
                log_type = LogType::Authentication.as_str(),
                "No receiver found for pending request - falling back to direct fetch"
            );
            self.handle_status_request(&req, &tmp_server, server_config)
                .await
        }
    }

    #[instrument(skip(self), fields(domain = %domain), level = "debug")]
    async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Finding server by domain: {}", domain
        );
        let configs = self
            .shared
            .configuration_service()
            .get_all_configurations()
            .await;
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Got {} total server configurations",
            configs.len()
        );

        let result = self
            .shared
            .configuration_service()
            .find_server_by_domain(domain)
            .await;

        debug!(
            domain = %domain,
            found = result.is_some(),
            "Domain lookup result"
        );

        if result.is_some() {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Found server for domain {}", domain
            );
        } else {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "No server found for domain {}", domain
            );
        }

        result
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        self.shared
            .configuration_service()
            .find_server_by_ip(ip)
            .await
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
            server_config.proxy_mode.unwrap_or_default()
        }
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

        if !req.is_login {
            let result = self.handle_status_request(&req, &tmp_server, server).await;
            return result;
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
            let result =
                self.create_status_response(req.domain.clone(), server, response, tmp_server);
            return result;
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
            proxy_mode: tmp_server.config.proxy_mode.unwrap_or_default(),
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
                .dial_with_proxy_protocol(req.session_id, req.client_addr, req.original_client_addr)
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
                    proxy_mode: tmp_server.config.proxy_mode.unwrap_or_default(),
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
        motd::generate_unreachable_motd_response(domain, server, self.shared.config())
    }

    async fn handle_unknown_server(&self, req: &ServerRequest) -> ProtocolResult<ServerResponse> {
        debug!("Handling unknown server for {}", req.domain);
        motd::generate_unknown_server_response(req.domain.clone(), self.shared.config())
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

                let result = self.handle_unknown_server(&req).await;
                return result;
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
