use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use log::{debug, info, warn};
use tokio::sync::{
    mpsc::{self},
    oneshot, Mutex, RwLock,
};
use wildmatch::WildMatch;

use crate::{
    core::{
        actors::{client::MinecraftClientHandler, server::MinecraftServerHandler},
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    Connection,
};

use super::{backend::Server, cache::StatusCache, ServerRequest, ServerRequester, ServerResponse};

pub struct Gateway {
    configurations: RwLock<HashMap<String, Arc<ServerConfig>>>,

    status_cache: Arc<Mutex<StatusCache>>,
    _sender: mpsc::Sender<GatewayMessage>,

    // The string represents the id provided by a Provider
    client_actors: RwLock<HashMap<String, std::sync::Mutex<Vec<MinecraftClientHandler>>>>,
    server_actors: RwLock<HashMap<String, std::sync::Mutex<Vec<MinecraftServerHandler>>>>,
}

#[derive(Clone)]
pub struct GatewayHandle {
    inner: Arc<Gateway>,
}

impl GatewayHandle {
    fn new(gateway: Arc<Gateway>) -> Self {
        Self { inner: gateway }
    }

    pub async fn request_server(&self, request: ServerRequest) -> ProtocolResult<ServerResponse> {
        self.inner.request_server(request).await
    }
}

impl Gateway {
    pub fn new(sender: mpsc::Sender<GatewayMessage>) -> Self {
        info!("Initializing ServerGateway");

        Self {
            configurations: RwLock::new(HashMap::new()),
            _sender: sender,
            client_actors: RwLock::new(HashMap::new()),
            server_actors: RwLock::new(HashMap::new()),
            status_cache: Arc::new(Mutex::new(StatusCache::new(Duration::from_secs(30)))),
        }
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<GatewayMessage>) {
        while let Some(message) = receiver.recv().await {
            match message {
                GatewayMessage::ConfigurationUpdate { key, configuration } => {
                    debug!("Configuration update received for key: {}", key);
                    self.update_configurations(vec![configuration]).await;

                    // Handle configuration updates
                }
                GatewayMessage::Shutdown => break,
                _ => {
                    debug!("Received unknown message: {:?}", message);
                }
            }
        }
    }

    pub async fn update_configurations(&self, configurations: Vec<ServerConfig>) {
        let mut config_lock = self.configurations.write().await;
        for config in configurations {
            config_lock.insert(config.config_id.clone(), Arc::new(config));
        }
    }

    pub async fn handle_client_connection(
        client_conn: Connection,
        request: ServerRequest,
        gateway: Arc<Gateway>,
    ) {
        let (server_sender, server_receiver) = mpsc::channel::<MinecraftCommunication>(64);
        let (client_sender, client_receiver) = mpsc::channel::<MinecraftCommunication>(64);
        let (oneshot_request_sender, oneshot_request_receiver) =
            oneshot::channel::<ServerResponse>();

        let server_config = match gateway.find_server(&request.domain).await {
            Some(server) => server,
            None => {
                warn!("Server not found for domain: {}", request.domain);
                return;
            }
        };

        let is_login = request.is_login.clone();

        // Create client actor
        let mut client_lock = gateway.client_actors.write().await;
        let serv_id = server_config.config_id.clone();
        let client_actor = client_lock
            .entry(serv_id.clone())
            .or_insert_with(|| std::sync::Mutex::new(vec![]));

        let client = MinecraftClientHandler::new(server_sender, client_receiver, client_conn, is_login);
        client_actor.lock().unwrap().push(client);

        // Create server actor
        let server = MinecraftServerHandler::new(
            client_sender,
            server_receiver,
            is_login,
            oneshot_request_receiver,
        );

        let mut server_lock = gateway.server_actors.write().await;
        let server_actor = server_lock
            .entry(serv_id.clone())
            .or_insert_with(|| std::sync::Mutex::new(vec![]));

        server_actor.lock().unwrap().push(server);

        // Spawn server request task
        let gateway_handle = gateway.clone();
        tokio::spawn(async move {
            match gateway_handle.wake_up_server(request, server_config).await {
                Ok(response) => {
                    debug!("Received server response: sending to client");
                    let _ = oneshot_request_sender.send(response);
                }
                Err(e) => {
                    warn!("Failed to request server: {:?}", e);
                }
            }
        });
    }

    async fn find_server(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        let domain = domain.to_lowercase();
        let time = std::time::Instant::now();
        warn!("Start waiting for lock in find_server : {}", domain);
        let serv_guard = self.configurations.read().await;
        let elapsed = time.elapsed();
        warn!(
            "End waiting for lock in find_server : {}, {}s",
            domain,
            elapsed.as_micros()
        );

        let result = serv_guard.values().cloned().into_iter().find(|server| {
            server.domains.iter().any(|pattern| {
                let matches = WildMatch::new(pattern).matches(&domain);
                debug!(
                    "Checking pattern '{}' against '{}': {}",
                    pattern, domain, matches
                );
                matches
            })
        });
        if result.is_none() {
            debug!(
                "Available patterns: {:?}",
                serv_guard.keys().collect::<Vec<_>>()
            );
        }

        result
    }

    pub async fn get_server_from_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        let serv_guard = self.configurations.read().await;
        serv_guard
            .iter()
            .find(|(_, server)| server.addresses.contains(&ip.to_string()))
            .map(|(_, server)| Arc::clone(server))
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
    use crate::{core::config::ServerConfig, proxy_modes::ProxyModeEnum};

    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

    fn setup_test_server() -> (TcpListener, String) {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr.to_string())
    }

    #[test]
    fn test_server_gateway() {
        let (_listener, addr) = setup_test_server();

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
