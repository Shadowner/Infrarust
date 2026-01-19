use std::sync::Arc;

use infrarust_config::{ServerConfig, models::logging::LogType};
use infrarust_server_manager::ServerState;
use tracing::{debug, error, instrument, warn};

use crate::{
    Connection,
    network::connection::PossibleReadValue,
    server::{
        ServerRequest, ServerResponse,
        motd::{MotdState, generate_response},
    },
};

use super::Gateway;

impl Gateway {
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

        let gateway = self.clone();
        tokio::spawn(async move {
            const STATUS_REQUEST_TIMEOUT_SECS: u64 = 10;

            let result = tokio::time::timeout(
                tokio::time::Duration::from_secs(STATUS_REQUEST_TIMEOUT_SECS),
                async {
                    let near_shutdown_threshold = 60;

                    let response: Result<ServerResponse, _> = match &server_config.server_manager {
                Some(config) => {
                    // Check if this server is near shutdown
                    let server_managers = gateway.shared.server_managers();
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
                            Arc::clone(&request.domain),
                            server_config.clone(),
                        )
                    } else {
                        let status = gateway
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
                                generate_response(MotdState::UnableToFetchStatus, Arc::clone(&request.domain), server_config)
                            }
                            Ok(server_status) => match server_status.state {
                                ServerState::Crashed => {
                                    warn!(
                                        "Server {} is crashed, using unreachable MOTD",
                                        server_config.config_id
                                    );
                                    generate_response(MotdState::Crashed, Arc::clone(&request.domain), server_config)
                                }
                                ServerState::Running => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is running", server_config.config_id
                                    );
                                    gateway
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
                                    generate_response(MotdState::Starting, Arc::clone(&request.domain), server_config)
                                }
                                ServerState::Stopped => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is stopped", server_config.config_id
                                    );
                                    generate_response(
                                        MotdState::Offline,
                                        Arc::clone(&request.domain),
                                        server_config,
                                    )
                                }
                                ServerState::Unknown => {
                                    error!(
                                        "Server {} is in unknown state",
                                        server_config.config_id
                                    );
                                    generate_response(MotdState::Crashed, Arc::clone(&request.domain), server_config)
                                }
                                ServerState::Stopping => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Server {} is stopping", server_config.config_id
                                    );
                                    generate_response(MotdState::Stopping, Arc::clone(&request.domain), server_config)
                                }
                            },
                        }
                    }
                }
                None => {
                    gateway
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

                        if let Err(e) = client.flush().await {
                            warn!(
                                log_type = LogType::Authentication.as_str(),
                                "Failed to flush status packet to client: {:?}", e
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

                                if let Err(e) = client.flush().await {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Failed to flush ping response: {:?}", e
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
                }
            ).await;

            if result.is_err() {
                warn!(
                    log_type = LogType::Authentication.as_str(),
                    "Status request timed out after {} seconds, forcing connection close",
                    STATUS_REQUEST_TIMEOUT_SECS
                );
            }

            if let Err(e) = client.close().await {
                warn!(
                    log_type = LogType::Authentication.as_str(),
                    "Error closing connection after status response: {:?}", e
                );
            }
        });
    }
}
