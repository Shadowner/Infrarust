use std::sync::Arc;

use log::debug;
use tokio::net::TcpStream;

use crate::{
    core::config::ServerConfig,
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    ServerConnection,
};

#[derive(Clone)]
pub struct Server {
    pub config: Arc<ServerConfig>,
}

impl Server {
    pub fn new(config: Arc<ServerConfig>) -> ProtocolResult<Self> {
        if config.addresses.is_empty() {
            return Err(ProxyProtocolError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "No server addresses configured",
            )));
        }
        Ok(Self { config })
    }

    pub async fn dial(&self) -> ProtocolResult<ServerConnection> {
        let mut last_error = None;



        debug!("Dialing server with ping: {:?}", self.config.addresses);

        for addr in &self.config.addresses {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    debug!("Connected to {}", addr);
                    stream.set_nodelay(true)?;
                    return Ok(ServerConnection::new(stream).await?);
                }
                Err(e) => {
                    debug!("Failed to connect to {}: {}", addr, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap().into())
    }
}
