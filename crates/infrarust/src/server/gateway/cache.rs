use std::sync::Arc;

use infrarust_config::{ServerConfig, models::logging::LogType};
use tokio::sync::watch;
use tracing::{debug, error};

use crate::{
    network::{packet::Packet, proxy_protocol::{ProtocolResult, errors::ProxyProtocolError}},
    server::{ServerRequest, ServerResponse, backend::Server, motd::MotdState},
};

use super::Gateway;

impl Gateway {
    /// Get or fetch status response with request coalescing
    pub(crate) async fn get_or_fetch_status_response(
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
                return self.generate_unreachable_motd_response(req.domain.to_string(), server_config);
            }
        };

        // Use consistent key for request deduplication
        let cache = self.status_cache.read().await;
        let key = cache.cache_key(&tmp_server, req.protocol_version);
        drop(cache);

        // Check if there's already a cached entry
        if let Some(packet) = self.try_quick_cache_lookup(&tmp_server, &req).await {
            return self.create_status_response(
                req.domain.to_string(),
                server_config,
                packet,
                &tmp_server,
            );
        }

        // Check for pending requests - if one exists, wait for it instead of making a new request
        let pending_receiver = {
            {
                let pending_requests = self.pending_status_requests.read().await;
                if let Some(receiver) = pending_requests.get(&key).cloned() {
                    // Another request is already in progress, wait for it
                    debug!(
                        "Waiting for in-progress status request for {} (key: {})",
                        req.domain, key
                    );
                    drop(pending_requests);
                    Some(receiver)
                } else {
                    drop(pending_requests);
                    // No pending request found with read lock, need to acquire write lock
                    let mut pending_requests = self.pending_status_requests.write().await;

                    // Double-check in case another task inserted while we were waiting for write lock
                    if let Some(receiver) = pending_requests.get(&key).cloned() {
                        Some(receiver)
                    } else {
                        // No pending request, create a new sender/receiver pair
                        let (sender, receiver) = watch::channel(None);
                        pending_requests.insert(key, receiver.clone());
                        drop(pending_requests);

                        // Spawn a task to fetch the status - clone required data for async move
                        let gateway = self.clone();
                        let server = tmp_server.clone();
                        let request = req.clone();
                        let config = Arc::clone(&server_config);

                        tokio::spawn(async move {
                            let result = match server.fetch_status_directly(&request).await {
                                Ok(packet) => {
                                    // Update cache
                                    let mut cache = gateway.status_cache.write().await;
                                    cache
                                        .update_cache_for(&server, &request, packet.clone())
                                        .await;

                                    if config.motds.online.is_some() {
                                        debug!(
                                            log_type = LogType::Authentication.as_str(),
                                            "Server reachable, using online MOTD for {}", request.domain
                                        );
                                        match crate::server::motd::generate_response(
                                            MotdState::Online,
                                            request.domain.to_string(),
                                            config.clone(),
                                        ) {
                                            Ok(resp) if resp.status_response.is_some() => Ok(resp.status_response.unwrap()),
                                            _ => Ok(packet), // Fallback to fetched packet if MOTD generation fails
                                        }
                                    } else {
                                        Ok(packet)
                                    }
                                }
                                Err(e) => {
                                    debug!(
                                        log_type = LogType::Authentication.as_str(),
                                        "Status fetch failed: {}. Using unreachable MOTD", e
                                    );
                                    // Get the error MOTD packet
                                    let motd_response = gateway.generate_unreachable_motd_response(
                                        request.domain.to_string(),
                                        config,
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
                            let mut pending_requests = gateway.pending_status_requests.write().await;
                            pending_requests.remove(&key);
                        });

                        Some(receiver)
                    }
                }
            }
        };

        // Wait for the pending request to complete
        if let Some(mut receiver) = pending_receiver {
            // Wait for the result to be available - only clone once when ready
            loop {
                if receiver.changed().await.is_err() {
                    // Sender dropped without sending result
                    debug!(
                        log_type = LogType::Authentication.as_str(),
                        "Watch channel sender dropped for {}", req.domain
                    );
                    return self.generate_unreachable_motd_response(req.domain.to_string(), server_config);
                }
                // Only clone when result is actually ready
                if let Some(result) = receiver.borrow().as_ref() {
                    return match result {
                        Ok(packet) => {
                            self.create_status_response(req.domain.to_string(), server_config, packet.clone(), &tmp_server)
                        }
                        Err(e) => Err(e.clone()),
                    };
                }
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

    pub(crate) async fn try_quick_cache_lookup(
        &self,
        tmp_server: &Server,
        req: &ServerRequest,
    ) -> Option<Packet> {
        match tokio::time::timeout(std::time::Duration::from_millis(100), async {
            let mut cache_guard = self.status_cache.write().await;
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

    pub(crate) fn update_cache_in_background(&self, tmp_server: &Server, req: &ServerRequest, packet: Packet) {
        let cache = Arc::clone(&self.status_cache);
        let tmp_server = tmp_server.clone();
        let req = req.clone();

        tokio::spawn(async move {
            if let Ok(mut cache_guard) = cache.try_write() {
                cache_guard
                    .update_cache_for(&tmp_server, &req, packet)
                    .await;
            }
        });
    }

    pub(crate) fn create_status_response(
        &self,
        domain: impl Into<String>,
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
            proxied_domain: Some(domain.into()),
            initial_config: server,
        })
    }

    pub(crate) fn generate_unreachable_motd_response(
        &self,
        domain: impl Into<String> + std::fmt::Display,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        debug!("Generating unreachable MOTD response for {}", domain);
        crate::server::motd::generate_response(MotdState::Unreachable, domain, server)
    }

    pub(crate) async fn handle_unknown_server(&self, req: &ServerRequest) -> ProtocolResult<ServerResponse> {
        debug!("Handling unknown server for {}", req.domain);
        let domain_str = req.domain.to_string();

        if let Some(motd) = self.shared.config().motds.unknown.clone() {
            let fake_config = Arc::new(ServerConfig {
                domains: vec![domain_str.clone()],
                addresses: vec![],
                config_id: format!("unknown_{}", domain_str),
                motds: infrarust_config::models::server::ServerMotds {
                    unknown: Some(motd),
                    ..Default::default()
                },
                ..ServerConfig::default()
            });
            crate::server::motd::generate_response(MotdState::Unknown, domain_str, fake_config)
        } else {
            Err(ProxyProtocolError::Other(format!(
                "Server not found for domain: {}",
                domain_str
            )))
        }
    }
}
