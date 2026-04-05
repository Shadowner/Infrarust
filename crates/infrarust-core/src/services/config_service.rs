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
    pub fn new(router: Arc<DomainRouter>) -> Self {
        Self { router }
    }

    /// Converts an internal [`infrarust_config::ServerConfig`] to an API [`ServerConfig`].
    fn convert_config(id: &str, config: &infrarust_config::ServerConfig) -> ServerConfig {
        ServerConfig::new(
            ServerId::new(id),
            config.network.clone(),
            config
                .addresses
                .iter()
                .map(|a| ServerAddress {
                    host: a.host.clone(),
                    port: a.port,
                })
                .collect(),
            config.domains.clone(),
            convert_proxy_mode(config.proxy_mode),
            config.limbo_handlers.clone(),
            config.max_players,
            config.disconnect_message.clone(),
            config.send_proxy_protocol,
            config.server_manager.is_some(),
        )
    }
}

impl infrarust_api::services::config_service::private::Sealed for ConfigServiceImpl {}

impl ConfigService for ConfigServiceImpl {
    fn get_server_config(&self, server: &ServerId) -> Option<ServerConfig> {
        let server_id = server.as_str();
        self.router
            .find_by_server_id(server_id)
            .map(|cfg| Self::convert_config(server_id, &cfg))
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
        _ => {
            tracing::warn!(
                ?mode,
                "unmapped ProxyMode variant, defaulting to Passthrough"
            );
            ProxyMode::Passthrough
        }
    }
}
