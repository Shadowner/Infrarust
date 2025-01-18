use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use log::{debug, info};
use tokio::sync::Mutex;
use wildmatch::WildMatch;

use crate::{
    core::config::ServerConfig,
    network::proxy_protocol::{errors::ProxyProtocolError, ProtocolResult},
};

use super::{backend::Server, cache::StatusCache, ServerRequest, ServerRequester, ServerResponse};

pub struct ServerGateway {
    servers: HashMap<String, Arc<Server>>,
    status_cache: Arc<Mutex<StatusCache>>,
}

impl ServerGateway {
    pub fn new(server_configs: Vec<ServerConfig>) -> Self {
        let mut server_map = HashMap::new();
        info!(
            "Initializing ServerGateway with {} servers",
            server_configs.len()
        );
        for configs in server_configs {
            let server = Arc::new(Server::new(configs).unwrap());

            for domain in &server.config.domains {
                debug!(
                    "Registering domain: {} -> {:?}",
                    domain, server.config.addresses
                );
                server_map.insert(domain.to_lowercase(), Arc::clone(&server));
            }
        }

        Self {
            servers: server_map,
            status_cache: Arc::new(Mutex::new(StatusCache::new(Duration::from_secs(30)))),
        }
    }

    fn find_server(&self, domain: &str) -> Option<Arc<Server>> {
        let domain = domain.to_lowercase();
        debug!("Looking for server matching domain: {}", domain);

        let result = self
            .servers
            .iter()
            .find(|(pattern, _)| {
                let matches = WildMatch::new(pattern).matches(&domain);
                debug!(
                    "Checking pattern '{}' against '{}': {}",
                    pattern, domain, matches
                );
                matches
            })
            .map(|(_, server)| Arc::clone(server));

        if result.is_none() {
            debug!(
                "Available patterns: {:?}",
                self.servers.keys().collect::<Vec<_>>()
            );
        }
        result
    }

    pub fn add_server(&mut self, server: Server) {
        for domain in &server.config.domains {
            self.servers
                .insert(domain.to_lowercase(), Arc::new(server.clone()));
        }
    }

    pub fn get_server_from_ip(&self, ip: &str) -> Option<Arc<Server>> {
        self.servers
            .values()
            .find(|server| server.config.addresses.contains(&ip.to_string()))
            .map(Arc::clone)
    }
}

#[async_trait]
impl ServerRequester for ServerGateway {
    async fn request_server(&self, req: ServerRequest) -> ProtocolResult<ServerResponse> {
        let server = self
            .find_server(&req.domain)
            .ok_or_else(|| ProxyProtocolError::Other("Server not found".to_string()))?;

        if req.is_login {
            let conn = server.dial().await?;
            Ok(ServerResponse {
                server_conn: conn,
                status_response: None,
                send_proxy_protocol: server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: req.read_packets.to_vec(),
                server_addr: req.client_addr.to_string().parse().ok(),
                proxy_mode: server.config.proxy_mode.clone().unwrap_or_default(), // Ajout du mode
                proxied_domain: Some(req.domain.clone()),
            })
        } else {
            let mut cache: tokio::sync::MutexGuard<'_, StatusCache> =
                self.status_cache.lock().await;
            let response = cache.get_status_response(&server, &req).await?;

            Ok(ServerResponse {
                server_conn: server.dial().await?,
                status_response: Some(response),
                send_proxy_protocol: server.config.send_proxy_protocol.unwrap_or_default(),
                read_packets: vec![], // No packets to forward
                server_addr: None,
                proxy_mode: server.config.proxy_mode.clone().unwrap_or_default(), // Ajout du mode
                proxied_domain: Some(req.domain.clone()),
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

        let server_config = ServerConfig {
            domains: vec!["example.com".to_string()],
            addresses: vec![addr],
            send_proxy_protocol: Some(false),
            proxy_mode: Some(ProxyModeEnum::Passthrough),
        };

        let gateway = ServerGateway::new(vec![server_config]);

        assert!(gateway.find_server("example.com").is_some());
        assert!(gateway.find_server("other.com").is_none());

        // TODO: Add more comprehensive tests for status caching and request handling
    }
    // Test server lookup
}
