use std::{fmt::Debug, sync::Arc};

use tokio::net::TcpStream;
use tracing::debug;
use uuid::Uuid;

use crate::{
    core::config::ServerConfig,
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
    telemetry::TELEMETRY,
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

    pub async fn dial(&self, session_id: Uuid) -> ProtocolResult<ServerConnection> {
        let mut last_error = None;
        debug!("Dialing server with ping: {:?}", self.config.addresses);

        for addr in &self.config.addresses {
            let now = std::time::Instant::now();
            TELEMETRY.record_backend_request_start(&self.config.config_id, &addr, &session_id);
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    debug!("Connected to {}", addr);
                    stream.set_nodelay(true)?;
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        true,
                        &session_id,
                        None,
                    );
                    return Ok(ServerConnection::new(stream, session_id).await?);
                }
                Err(e) => {
                    debug!("Failed to connect to {}: {}", addr, e);
                    TELEMETRY.record_backend_request_end(
                        &self.config.config_id,
                        addr,
                        now,
                        false,
                        &session_id,
                        Some(&e),
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap().into())
    }
}
