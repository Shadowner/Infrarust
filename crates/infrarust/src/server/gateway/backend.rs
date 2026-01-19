use std::sync::Arc;

use infrarust_config::{ServerConfig, models::logging::LogType};
use tracing::{debug, error, instrument};

use crate::{
    network::proxy_protocol::ProtocolResult,
    server::{ServerRequest, ServerResponse, backend::Server},
};

use super::Gateway;

impl Gateway {
    #[instrument(name = "wake_up_server_internal", skip(self, req, server), fields(
        domain = %req.domain,
        is_login = %req.is_login,
        server_addr = %server.addresses.first().unwrap_or(&String::new()),
        session_id = %req.session_id
    ))]
    pub(crate) async fn wake_up_server_internal(
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
                return self.generate_unreachable_motd_response(Arc::clone(&req.domain), server);
            }
        };

        if !req.is_login {
            let result = self.handle_status_request(&req, &tmp_server, server).await;
            return result;
        }

        debug!("Creating login connection to backend server");

        self.handle_login_request(&req, &tmp_server, server).await
    }

    pub(crate) async fn handle_status_request(
        &self,
        req: &ServerRequest,
        tmp_server: &Server,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        debug!("Fast-path for status request to {}", req.domain);

        if let Some(response) = self.try_quick_cache_lookup(tmp_server, req).await {
            let result =
                self.create_status_response(Arc::clone(&req.domain), server, response, tmp_server);
            return result;
        }

        debug!("No quick cache hit, fetching status directly from server");
        match tmp_server.fetch_status_directly(req).await {
            Ok(packet) => {
                // Update cache in the background without waiting
                self.update_cache_in_background(tmp_server, req, packet.clone());

                self.create_status_response(Arc::clone(&req.domain), server, packet, tmp_server)
            }
            Err(e) => {
                debug!("Status fetch failed: {}. Using unreachable MOTD", e);
                self.generate_unreachable_motd_response(Arc::clone(&req.domain), server)
            }
        }
    }

    pub(crate) async fn handle_login_request(
        &self,
        req: &ServerRequest,
        tmp_server: &Server,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let use_proxy_protocol = tmp_server.config.send_proxy_protocol.unwrap_or(false);
        let conn = if use_proxy_protocol {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Using proxy protocol for connection"
            );
            tmp_server
                .dial_with_proxy_protocol(req.session_id, req.client_addr, req.original_client_addr)
                .await
        } else {
            debug!(
                log_type = LogType::Authentication.as_str(),
                "Using standard connection"
            );
            tmp_server.dial(req.session_id).await
        };

        match conn {
            Ok(connection) => {
                debug!(
                    log_type = LogType::Authentication.as_str(),
                    "Login connection established successfully"
                );
                Ok(ServerResponse {
                    server_conn: Some(connection),
                    status_response: None,
                    send_proxy_protocol: use_proxy_protocol,
                    read_packets: req.read_packets.to_vec(),
                    server_addr: Some(req.client_addr),
                    proxy_mode: tmp_server.config.proxy_mode.unwrap_or_default(),
                    proxied_domain: Some(Arc::clone(&req.domain)),
                    initial_config: server,
                })
            }
            Err(e) => {
                debug!(
                    log_type = LogType::Authentication.as_str(),
                    "Failed to connect to backend server: {}", e
                );
                Err(e)
            }
        }
    }
}
