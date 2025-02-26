use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::sync::{
    mpsc::{self},
    oneshot, Mutex,
};
use tracing::{debug, debug_span, info, instrument, warn, Instrument, Span};

use crate::{
    core::{actors::supervisor::ActorSupervisor, config::ServerConfig, event::GatewayMessage},
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    protocol::minecraft::java::login::ServerBoundLoginStart,
    proxy_modes::ProxyModeEnum,
    Connection,
};

use super::{backend::Server, cache::StatusCache, ServerRequest, ServerRequester, ServerResponse};
use crate::core::config::service::ConfigurationService;
use crate::telemetry::TELEMETRY;

pub struct Gateway {
    config_service: Arc<ConfigurationService>,
    status_cache: Arc<Mutex<StatusCache>>,
    _sender: mpsc::Sender<GatewayMessage>,
    actor_supervisor: Arc<ActorSupervisor>,
}

impl Gateway {
    pub fn new(
        sender: mpsc::Sender<GatewayMessage>,
        config_service: Arc<ConfigurationService>,
    ) -> Self {
        info!("Initializing ServerGateway");

        Self {
            config_service,
            _sender: sender,
            actor_supervisor: Arc::new(ActorSupervisor::new()),
            status_cache: Arc::new(Mutex::new(StatusCache::new(Duration::from_secs(30)))),
        }
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<GatewayMessage>) {
        //TODO: For future use
        // Keep the gateway running until a shutdown message is received
        #[allow(clippy::never_loop)]
        while let Some(message) = receiver.recv().await {
            match message {
                GatewayMessage::Shutdown => break,
            }
        }
    }

    pub async fn update_configurations(&self, configurations: Vec<ServerConfig>) {
        self.config_service
            .update_configurations(configurations)
            .await;
    }

    pub async fn remove_configuration(&self, config_id: &str) {
        self.config_service.remove_configuration(config_id).await;
    }

    #[instrument(name = "client_connection_handling", skip(client, request, gateway), fields(
        domain = %request.domain,
        is_login = request.is_login,
        protocol_version = ?request.protocol_version,
        client_addr = %request.client_addr
    ))]
    pub async fn handle_client_connection(
        client: Connection,
        request: ServerRequest,
        gateway: Arc<Gateway>,
    ) {
        let span = Span::current();

        let server_config = match gateway
            .find_server(&request.domain)
            .instrument(span.clone())
            .await
        {
            Some(server) => server,
            None => {
                warn!("Server not found for domain: {}", request.domain);
                return;
            }
        };

        let is_login = request.is_login;
        let proxy_mode = if !is_login {
            TELEMETRY.record_request();
            ProxyModeEnum::Status
        } else {
            TELEMETRY.record_new_connection(
                &request.client_addr.to_string(),
                &request.domain,
                request.session_id,
            );
            server_config.proxy_mode.clone().unwrap_or_default()
        };

        let username = if is_login {
            let login_start = &request.read_packets[1];
            ServerBoundLoginStart::try_from(login_start).unwrap().name.0
        } else {
            String::new()
        };

        let (oneshot_request_sender, oneshot_request_receiver) = oneshot::channel();

        // Créer les acteurs avec le span parent
        gateway
            .actor_supervisor
            .create_actor_pair(
                &server_config.config_id,
                client,
                proxy_mode.clone(),
                oneshot_request_receiver,
                is_login,
                username.clone(),
            )
            .instrument(debug_span!(parent: span.clone(), "create_actors",
                username = %username,
                proxy_mode = ?proxy_mode
            ))
            .await;

        // Wake up server dans le même span
        tokio::spawn(
            async move {
                match gateway.wake_up_server(request, server_config).await {
                    Ok(response) => {
                        let _ = oneshot_request_sender.send(response);
                    }
                    Err(e) => {
                        warn!("Failed to request server: {:?}", e);
                    }
                }
            }
            .instrument(span),
        );
    }

    async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        let span = debug_span!("gateway: find_server", domain = domain);
        self.config_service
            .find_server_by_domain(domain)
            .instrument(span)
            .await
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        self.config_service.find_server_by_ip(ip).await
    }
}

#[async_trait]
impl ServerRequester for Gateway {
    #[instrument(name = "request_server", skip(self, req), fields(
        domain = %req.domain,
        is_login = req.is_login
    ))]
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse> {
        let server_config = self
            .find_server(&req.domain)
            .instrument(debug_span!("server_request: find_server"))
            .await
            .ok_or_else(|| ProxyProtocolError::Other("Server not found".to_string()))?;

        self.wake_up_server(req, server_config)
            .instrument(debug_span!("server_request: wake_up_server"))
            .await
    }

    #[instrument(name = "wake_up_server", skip(self, req, server), fields(
        domain = %req.domain,
        is_login = %req.is_login,
        server_addr = %server.addresses.first().unwrap_or(&String::new())
    ))]
    async fn wake_up_server(
        &self,
        req: ServerRequest,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let tmp_server = Server::new(server.clone())?;

        if req.is_login {
            debug!("Creating login connection to backend server");
            let conn = if tmp_server.config.send_proxy_protocol.unwrap_or(false) {
                debug!("Using proxy protocol for connection");
                tmp_server
                    .dial_with_proxy_protocol(req.session_id, req.client_addr)
                    .await?
            } else {
                debug!("Using standard connection");
                tmp_server.dial(req.session_id).await?
            };

            Ok(ServerResponse {
                server_conn: Some(conn),
                status_response: None,
                send_proxy_protocol: tmp_server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: req.read_packets.to_vec(),
                server_addr: Some(req.client_addr),
                proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(),
                proxied_domain: Some(req.domain.clone()),
                initial_config: server.clone(),
            })
        } else {
            debug!("Fetching status from cache or backend");
            let mut cache = self.status_cache.lock().await;

            let response = cache
                .get_status_response(&tmp_server, &req)
                .instrument(debug_span!("status_cache_lookup"))
                .await?;

            Ok(ServerResponse {
                server_conn: None,
                status_response: Some(response),
                send_proxy_protocol: tmp_server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: vec![], // No packets to forward
                server_addr: None,
                proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(),
                proxied_domain: Some(req.domain.clone()),
                initial_config: server.clone(),
            })
        }
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
    // Test server lookup
}
