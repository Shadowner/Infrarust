pub mod backend;
pub mod cache;
pub mod gateway;
pub mod manager;
pub mod motd;

use crate::ServerConnection;
use crate::network::packet::Packet;
use crate::network::proxy_protocol::ProtocolResult;
use async_trait::async_trait;
use infrarust_config::ServerConfig;
use infrarust_config::models::server::ProxyModeEnum;
use infrarust_protocol::version::Version;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ServerRequest {
    pub client_addr: SocketAddr,
    pub original_client_addr: Option<SocketAddr>,

    pub domain: Arc<str>,
    pub is_login: bool,
    pub protocol_version: Version,
    pub read_packets: Arc<[Packet; 2]>,
    pub session_id: uuid::Uuid,
}

#[derive(Debug)]
pub struct ServerResponse {
    pub server_conn: Option<ServerConnection>,
    pub status_response: Option<Packet>,
    pub send_proxy_protocol: bool,
    pub read_packets: Vec<Packet>,
    pub server_addr: Option<SocketAddr>,
    pub proxy_mode: ProxyModeEnum,
    pub proxied_domain: Option<Arc<str>>,
    pub initial_config: Arc<ServerConfig>,
}

impl Default for ServerResponse {
    fn default() -> Self {
        Self {
            server_conn: None,
            status_response: None,
            send_proxy_protocol: false,
            read_packets: Vec::new(),
            server_addr: None,
            proxy_mode: ProxyModeEnum::default(),
            proxied_domain: None,
            initial_config: Arc::new(ServerConfig::default()),
        }
    }
}

#[async_trait]
pub trait ServerRequester: Send + Sync {
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse>;

    async fn wake_up_server(
        &self,
        req: ServerRequest,
        server: Arc<ServerConfig>,
    ) -> ProtocolResult<ServerResponse>;
}
