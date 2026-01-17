use std::sync::Arc;

use async_trait::async_trait;
use infrarust_config::ServerConfig;
use tracing::{Instrument, debug, debug_span, instrument};

use crate::{
    network::proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    server::{ServerRequest, ServerRequester, ServerResponse},
};

use super::Gateway;

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
        debug!("Wake up server: {} with {}", &req.domain, &server.config_id);
        let domain = req.domain.clone();
        let result = self.wake_up_server_internal(req, server).await;
        match &result {
            Ok(_) => debug!("Wake up server successful for: {}", domain),
            Err(e) => debug!("Wake up server failed for: {}: {}", domain, e),
        }
        result
    }
}
