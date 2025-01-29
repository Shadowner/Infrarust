use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use log::{info, warn};
use tokio::sync::{
    mpsc::{self},
    oneshot, Mutex,
};

use crate::{
    core::{actors::supervisor::ActorSupervisor, config::ServerConfig, event::GatewayMessage},
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    protocol::minecraft::java::login::ServerBoundLoginStart,
    proxy_modes::ProxyModeEnum,
    Connection,
};

use super::{backend::Server, cache::StatusCache, ServerRequest, ServerRequester, ServerResponse};
use crate::core::config::service::ConfigurationService;

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

    pub async fn handle_client_connection(
        client_conn: Connection,
        request: ServerRequest,
        gateway: Arc<Gateway>,
    ) {
        let (oneshot_request_sender, oneshot_request_receiver) =
            oneshot::channel::<ServerResponse>();

        let server_config = match gateway.find_server(&request.domain).await {
            Some(server) => server,
            None => {
                warn!("Server not found for domain: {}", request.domain);
                return;
            }
        };

        let is_login = request.is_login;
        let mut proxy_mode = server_config.proxy_mode.clone().unwrap_or_default();
        let mut username = String::new();
        if !is_login {
            proxy_mode = ProxyModeEnum::Status;
        } else {
            let login_start = &request.read_packets[1];
            username = ServerBoundLoginStart::try_from(login_start).unwrap().name.0;
        }

        gateway
            .actor_supervisor
            .create_actor_pair(
                &server_config.config_id,
                client_conn,
                proxy_mode,
                oneshot_request_receiver,
                is_login,
                username,
            )
            .await;

        // Spawn server request task
        let gateway_handle = gateway.clone();
        tokio::spawn(async move {
            match gateway_handle.wake_up_server(request, server_config).await {
                Ok(response) => {
                    let _ = oneshot_request_sender.send(response);
                }
                Err(e) => warn!("Failed to request server: {:?}", e),
            }
        });
    }

    async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        self.config_service.find_server_by_domain(domain).await
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        self.config_service.find_server_by_ip(ip).await
    }
}

#[async_trait]
impl ServerRequester for Gateway {
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse> {
        let server_config = self
            .find_server(&req.domain)
            .await
            .ok_or_else(|| ProxyProtocolError::Other("Server not found".to_string()))?;

        self.wake_up_server(req, server_config).await
    }

    async fn wake_up_server(
        &self,
        req: ServerRequest,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse> {
        let tmp_server = Server::new(server.clone())?;

        if req.is_login {
            let conn = tmp_server.dial().await?;
            Ok(ServerResponse {
                server_conn: Some(conn),
                status_response: None,
                send_proxy_protocol: tmp_server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: req.read_packets.to_vec(),
                server_addr: req.client_addr.to_string().parse().ok(),
                proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(), // Ajout du mode
                proxied_domain: Some(req.domain.clone()),
                initial_config: server.clone(),
            })
        } else {
            let mut cache = self.status_cache.lock().await; //TODO: This may cause a deadlock

            let response = cache.get_status_response(&tmp_server, &req).await?;

            Ok(ServerResponse {
                server_conn: None,
                status_response: Some(response),
                send_proxy_protocol: tmp_server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: vec![], // No packets to forward
                server_addr: None,
                proxy_mode: tmp_server.config.proxy_mode.clone().unwrap_or_default(), // Ajout du mode
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
