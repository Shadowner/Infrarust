use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use async_trait::async_trait;
use log::{debug, info, warn};
use tokio::sync::{
    mpsc::{self},
    oneshot, Mutex, RwLock,
};

use crate::{
    core::{
        actors::{client::MinecraftClientHandler, server::MinecraftServerHandler},
        config::ServerConfig,
        event::{GatewayMessage, MinecraftCommunication},
    },
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    protocol::minecraft::java::login::ServerBoundLoginStart,
    proxy_modes::{
        client_only::{ClientOnlyMessage, ClientOnlyMode},
        offline::{OfflineMessage, OfflineMode},
        passthrough::{PassthroughMessage, PassthroughMode},
        status::{StatusMessage, StatusMode},
        ClientProxyModeHandler, ProxyModeEnum, ServerProxyModeHandler,
    },
    Connection,
};

use super::{backend::Server, cache::StatusCache, ServerRequest, ServerRequester, ServerResponse};
use crate::core::config::service::ConfigurationService;

//TODO: In the future I think this will be replaced by a more generic actor system
// For plugin handling
type ClientActorMap =
    HashMap<String, std::sync::Mutex<Vec<(MinecraftClientHandler, Arc<AtomicBool>)>>>;
type ServerActorMap =
    HashMap<String, std::sync::Mutex<Vec<(MinecraftServerHandler, Arc<AtomicBool>)>>>;

pub struct Gateway {
    config_service: Arc<ConfigurationService>,
    status_cache: Arc<Mutex<StatusCache>>,
    _sender: mpsc::Sender<GatewayMessage>,

    // The string represents the id provided by a Provider
    client_actors: RwLock<ClientActorMap>,
    server_actors: RwLock<ServerActorMap>,
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
            client_actors: RwLock::new(HashMap::new()),
            server_actors: RwLock::new(HashMap::new()),
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
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let is_login = request.is_login;
        if !is_login {
            debug!("Handling status request for domain: {}", request.domain);
            let c_mode: Box<dyn ClientProxyModeHandler<MinecraftCommunication<StatusMessage>>> =
                Box::new(StatusMode);
            let s_mode: Box<dyn ServerProxyModeHandler<MinecraftCommunication<StatusMessage>>> =
                Box::new(StatusMode);
            let (s_tx, s_rx) = mpsc::channel(1);
            let (c_tx, c_rx) = mpsc::channel(1);

            MinecraftClientHandler::new(
                s_tx,
                c_rx,
                c_mode,
                client_conn,
                is_login,
                "".to_string(),
                shutdown_flag.clone(),
            );
            MinecraftServerHandler::new(
                c_tx,
                s_rx,
                is_login,
                oneshot_request_receiver,
                s_mode,
                shutdown_flag.clone(),
            );

            let gateway_handle = gateway.clone();
            tokio::spawn(async move {
                match gateway_handle.wake_up_server(request, server_config).await {
                    Ok(response) => {
                        debug!("Received server response: sending to Server Actor");
                        let _ = oneshot_request_sender.send(response);
                    }
                    Err(e) => {
                        warn!("Failed to request server: {:?}", e);
                    }
                }
            });

            return;
        }
        let login_start = &request.read_packets[1];
        //TODO: Match instead of unwrap
        let username = ServerBoundLoginStart::try_from(login_start).unwrap().name.0;

        match server_config.proxy_mode.clone().unwrap() {
            ProxyModeEnum::Passthrough => {
                let client_handler: Box<
                    dyn ClientProxyModeHandler<MinecraftCommunication<PassthroughMessage>>,
                > = Box::new(PassthroughMode);
                let server_handler: Box<
                    dyn ServerProxyModeHandler<MinecraftCommunication<PassthroughMessage>>,
                > = Box::new(PassthroughMode);
                let (server_sender, server_receiver) =
                    mpsc::channel::<MinecraftCommunication<PassthroughMessage>>(64);
                let (client_sender, client_receiver) =
                    mpsc::channel::<MinecraftCommunication<PassthroughMessage>>(64);

                let client = MinecraftClientHandler::new(
                    server_sender,
                    client_receiver,
                    client_handler,
                    client_conn,
                    is_login,
                    username,
                    shutdown_flag.clone(),
                );

                let server = MinecraftServerHandler::new(
                    client_sender,
                    server_receiver,
                    is_login,
                    oneshot_request_receiver,
                    server_handler,
                    shutdown_flag.clone(),
                );

                let config_id = server_config.config_id.clone();
                let gateway_handle = gateway.clone();
                tokio::spawn(async move {
                    gateway_handle
                        .register_client_actor(&config_id, client, shutdown_flag.clone())
                        .await;
                    gateway_handle
                        .register_server_actor(&config_id, server, shutdown_flag.clone())
                        .await;
                });
            }

            ProxyModeEnum::Offline => {
                let client_handler: Box<
                    dyn ClientProxyModeHandler<MinecraftCommunication<OfflineMessage>>,
                > = Box::new(OfflineMode);
                let server_handler: Box<
                    dyn ServerProxyModeHandler<MinecraftCommunication<OfflineMessage>>,
                > = Box::new(OfflineMode);
                let (server_sender, server_receiver) =
                    mpsc::channel::<MinecraftCommunication<OfflineMessage>>(64);
                let (client_sender, client_receiver) =
                    mpsc::channel::<MinecraftCommunication<OfflineMessage>>(64);

                let client = MinecraftClientHandler::new(
                    server_sender,
                    client_receiver,
                    client_handler,
                    client_conn,
                    is_login,
                    username,
                    shutdown_flag.clone(),
                );

                let server = MinecraftServerHandler::new(
                    client_sender,
                    server_receiver,
                    is_login,
                    oneshot_request_receiver,
                    server_handler,
                    shutdown_flag.clone(),
                );

                let config_id = server_config.config_id.clone();
                let gateway_handle = gateway.clone();
                tokio::spawn(async move {
                    gateway_handle
                        .register_client_actor(&config_id, client, shutdown_flag.clone())
                        .await;
                    gateway_handle
                        .register_server_actor(&config_id, server, shutdown_flag.clone())
                        .await;
                });
            }
            ProxyModeEnum::ClientOnly => {
                let client_handler: Box<
                    dyn ClientProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>>,
                > = Box::new(ClientOnlyMode);
                let server_handler: Box<
                    dyn ServerProxyModeHandler<MinecraftCommunication<ClientOnlyMessage>>,
                > = Box::new(ClientOnlyMode);
                let (server_sender, server_receiver) =
                    mpsc::channel::<MinecraftCommunication<ClientOnlyMessage>>(64);
                let (client_sender, client_receiver) =
                    mpsc::channel::<MinecraftCommunication<ClientOnlyMessage>>(64);

                let client = MinecraftClientHandler::new(
                    server_sender,
                    client_receiver,
                    client_handler,
                    client_conn,
                    is_login,
                    username,
                    shutdown_flag.clone(),
                );

                let server = MinecraftServerHandler::new(
                    client_sender,
                    server_receiver,
                    is_login,
                    oneshot_request_receiver,
                    server_handler,
                    shutdown_flag.clone(),
                );

                let config_id = server_config.config_id.clone();
                let gateway_handle = gateway.clone();
                tokio::spawn(async move {
                    gateway_handle
                        .register_client_actor(&config_id, client, shutdown_flag.clone())
                        .await;
                    gateway_handle
                        .register_server_actor(&config_id, server, shutdown_flag.clone())
                        .await;
                });
            }
            ProxyModeEnum::ServerOnly => {
                panic!("ServerOnly mode not implemented yet");
            }
        };

        // Spawn server request task
        let gateway_handle = gateway.clone();
        tokio::spawn(async move {
            match gateway_handle.wake_up_server(request, server_config).await {
                Ok(response) => {
                    debug!("Received server response: sending to Server Actor");
                    let _ = oneshot_request_sender.send(response);
                }
                Err(e) => {
                    warn!("Failed to request server: {:?}", e);
                }
            }
        });
    }

    async fn register_client_actor(
        &self,
        server_id: &str,
        client: MinecraftClientHandler,
        shutdown: Arc<AtomicBool>,
    ) {
        let mut client_lock = self.client_actors.write().await;
        let client_actor = client_lock
            .entry(server_id.to_string())
            .or_insert_with(|| std::sync::Mutex::new(vec![]));

        client_actor.get_mut().unwrap().push((client, shutdown));
    }

    async fn register_server_actor(
        &self,
        server_id: &str,
        server: MinecraftServerHandler,
        shutdown: Arc<AtomicBool>,
    ) {
        let mut server_lock = self.server_actors.write().await;
        let server_actor = server_lock
            .entry(server_id.to_string())
            .or_insert_with(|| std::sync::Mutex::new(vec![]));

        server_actor.get_mut().unwrap().push((server, shutdown));
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
