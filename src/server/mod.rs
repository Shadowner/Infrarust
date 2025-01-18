pub mod backend;
pub mod cache;
pub mod gateway;

use crate::network::packet::Packet;
use crate::network::proxy_protocol::ProtocolResult;
use crate::protocol::version::Version;
use crate::proxy_modes::ProxyModeEnum;
use crate::ServerConnection;
use async_trait::async_trait;
use std::net::SocketAddr;

pub struct ServerRequest {
    pub client_addr: SocketAddr,
    pub domain: String,
    pub is_login: bool,
    pub protocol_version: Version,
    pub read_packets: [Packet; 2],
}

pub struct ServerResponse {
    pub server_conn: ServerConnection,
    pub status_response: Option<Packet>,
    pub send_proxy_protocol: bool,
    pub read_packets: Vec<Packet>,
    pub server_addr: Option<SocketAddr>,
    pub proxy_mode: ProxyModeEnum,
    pub proxied_domain: Option<String>,
}

#[async_trait]
pub trait ServerRequester: Send + Sync {
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse>;
}
