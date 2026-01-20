mod backend;
mod cache;
mod connection;
mod lookup;
mod requester;
mod status;

use std::{collections::HashMap, sync::Arc};

use infrarust_config::{ServerConfig, models::logging::LogType};
use tokio::sync::{
    RwLock,
    mpsc::{self},
    watch::Receiver,
};
use tracing::{debug, info};

use crate::{
    core::{event::GatewayMessage, shared_component::SharedComponent},
    network::proxy_protocol::errors::ProxyProtocolError,
    server::cache::StatusCache,
};

pub const HEALTH_CHECK_INTERVAL_SECS: u64 = 60;

#[derive(Debug, Clone)]
pub struct GatewayMetrics {
    pub pending_status_requests_count: usize,
    pub status_cache_entries: usize,
    pub status_cache_max_size: usize,
}

#[derive(Debug, Clone)]
pub struct Gateway {
    status_cache: Arc<RwLock<StatusCache>>,
    pub(crate) shared: Arc<SharedComponent>,
    #[allow(clippy::type_complexity)]
    pending_status_requests: Arc<
        RwLock<
            HashMap<
                u64,
                Receiver<Option<Result<Arc<crate::network::packet::Packet>, ProxyProtocolError>>>,
            >,
        >,
    >,
}

impl Gateway {
    pub fn new(shared: Arc<SharedComponent>) -> Self {
        info!(
            log_type = LogType::Authentication.as_str(),
            "Initializing ServerGateway"
        );

        let config = shared.config();
        let gateway = Self {
            status_cache: Arc::new(RwLock::new(StatusCache::from_shared_config(config))),
            pending_status_requests: Arc::new(RwLock::new(HashMap::new())),
            shared,
        };

        let supervisor = gateway.shared.actor_supervisor_arc();
        let shutdown = gateway.shared.shutdown_controller_arc();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));
            let mut shutdown_rx = shutdown.subscribe().await;

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!(log_type = LogType::Authentication.as_str(), "Health check task received shutdown signal");
                        break;
                    }
                    _ = interval.tick() => {
                        supervisor.health_check().await;
                        supervisor.check_and_mark_empty_servers().await;
                    }
                }
            }
        });

        gateway
    }

    pub async fn run(&self, mut receiver: mpsc::Receiver<GatewayMessage>) {
        // Keep the gateway running until a shutdown message is received.
        // Currently only handles Shutdown; more message types can be added as needed.
        #[allow(clippy::never_loop)]
        while let Some(message) = receiver.recv().await {
            match message {
                GatewayMessage::Shutdown => {
                    debug!(
                        log_type = LogType::Authentication.as_str(),
                        "Gateway received shutdown message"
                    );
                    break;
                }
            }
        }
        debug!(
            log_type = LogType::Authentication.as_str(),
            "Gateway run loop exited"
        );
    }

    pub async fn update_configurations(&self, configurations: Vec<ServerConfig>) {
        self.shared
            .configuration_service()
            .update_configurations(configurations)
            .await;
    }

    pub async fn remove_configuration(&self, config_id: &str) {
        self.shared
            .configuration_service()
            .remove_configuration(config_id)
            .await;
    }

    pub fn get_memory_metrics(&self) -> Option<GatewayMetrics> {
        let pending = self.pending_status_requests.try_read().ok()?;
        let cache = self.status_cache.try_read().ok()?;

        Some(GatewayMetrics {
            pending_status_requests_count: pending.len(),
            status_cache_entries: cache.entry_count(),
            status_cache_max_size: cache.max_size(),
        })
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
}
