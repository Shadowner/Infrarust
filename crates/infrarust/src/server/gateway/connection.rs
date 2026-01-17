use infrarust_config::{ServerConfig, models::{logging::LogType, server::ProxyModeEnum}};
use infrarust_protocol::minecraft::java::login::ServerBoundLoginStart;
use infrarust_server_manager::ServerState;
use tokio::sync::oneshot;
use tracing::{Instrument, Span, debug, debug_span, info, instrument, warn};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;
use crate::{Connection, security::BanHelper, server::{ServerRequest, ServerRequester}};

use super::Gateway;

impl Gateway {
    async fn is_username_banned(&self, username: &str) -> Option<String> {
        BanHelper::is_username_banned(self.shared.filter_registry(), username).await
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
            "Looking up server for domain: {}", request.domain
        );
        let server_config = self.find_server(&request.domain).await;
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
                let server_id = &manager_config.server_id;
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
                        .start_server(server_id, manager_type)
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
                        .remove_server_from_empty(server_id, manager_type)
                        .await;
                }
            }
        }

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
                &request.domain,
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

        let supervisor = self.shared.actor_supervisor_arc();
        let server_config_clone = server_config.clone();
        let connecting_domain = request.domain.clone();

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

    pub(crate) fn extract_username_from_request(request: &ServerRequest) -> Result<String, String> {
        let login_start = &request.read_packets[1];
        ServerBoundLoginStart::try_from(login_start)
            .map(|login| login.name.0.clone())
            .map_err(|e| format!("{:?}", e))
    }

    pub(crate) fn determine_proxy_mode(
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
}
