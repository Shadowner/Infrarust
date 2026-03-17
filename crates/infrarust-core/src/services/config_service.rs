//! [`ConfigService`] implementation — read-only access to proxy configuration.

use std::sync::Arc;

use infrarust_api::services::config_service::{ConfigService, ProxyMode, ServerConfig};
use infrarust_api::types::{ServerAddress, ServerId};

use crate::routing::DomainRouter;

/// Read-only wrapper around the proxy's configuration and routing tables.
pub struct ConfigServiceImpl {
    router: Arc<DomainRouter>,
}

impl ConfigServiceImpl {
    /// Creates a new config service backed by the given router.
    pub fn new(router: Arc<DomainRouter>) -> Self {
        Self { router }
    }

    /// Converts an internal [`infrarust_config::ServerConfig`] to an API [`ServerConfig`].
    fn convert_config(
        id: &str,
        config: &infrarust_config::ServerConfig,
    ) -> ServerConfig {
        ServerConfig {
            id: ServerId::new(id),
            addresses: config
                .addresses
                .iter()
                .map(|a| ServerAddress {
                    host: a.host.clone(),
                    port: a.port,
                })
                .collect(),
            domains: config.domains.clone(),
            proxy_mode: convert_proxy_mode(config.proxy_mode),
            limbo_handlers: config.limbo_handlers.clone(),
        }
    }
}

impl infrarust_api::services::config_service::private::Sealed for ConfigServiceImpl {}

impl ConfigService for ConfigServiceImpl {
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
        let server_id = server.as_str();
        self.router
            .list_all()
            .into_iter()
            .find(|(_, cfg)| cfg.effective_id() == server_id)
            .map(|(_, cfg)| Self::convert_config(server_id, &cfg))
    }

    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        self.router
            .list_all()
            .into_iter()
            .map(|(_, cfg)| {
                let id = cfg.effective_id();
                Self::convert_config(&id, &cfg)
            })
            .collect()
    }

    fn get_value(&self, _key: &str) -> Option<String> {
        None // Phase future
    }
}

/// Converts internal proxy mode to API proxy mode.
fn convert_proxy_mode(mode: infrarust_config::ProxyMode) -> ProxyMode {
    match mode {
        infrarust_config::ProxyMode::Passthrough => ProxyMode::Passthrough,
        infrarust_config::ProxyMode::ZeroCopy => ProxyMode::ZeroCopy,
        infrarust_config::ProxyMode::ClientOnly => ProxyMode::ClientOnly,
        infrarust_config::ProxyMode::Offline => ProxyMode::Offline,
        infrarust_config::ProxyMode::ServerOnly => ProxyMode::ServerOnly,
        _ => ProxyMode::Passthrough,
    }
}
