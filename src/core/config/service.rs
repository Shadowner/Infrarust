use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{debug, debug_span, instrument, Instrument};
use wildmatch::WildMatch;

use crate::{core::config::ServerConfig, telemetry::TELEMETRY};

#[derive(Clone)]
pub struct ConfigurationService {
    configurations: Arc<RwLock<HashMap<String, Arc<ServerConfig>>>>,
}

impl Default for ConfigurationService {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigurationService {
    #[instrument(name = "create_config_service")]
    pub fn new() -> Self {
        debug!("Creating new configuration service");
        Self {
            configurations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[instrument(skip(self), fields(domain = %domain))]
    pub async fn find_server_by_domain(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!("Finding server by domain");
        let domain = domain.to_lowercase();
        let configs = self.configurations.read().await;
        let result = configs
            .values()
            .find(|server| {
                server
                    .domains
                    .iter()
                    .any(|pattern| WildMatch::new(pattern).matches(&domain))
            })
            .cloned();

        debug!(found = result.is_some(), "Domain lookup result");
        result
    }

    #[instrument(skip(self), fields(ip = %ip))]
    pub async fn find_server_by_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        debug!("Finding server by IP");
        let configs = self.configurations.read().await;
        let result = configs
            .iter()
            .find(|(_, server)| server.addresses.contains(&ip.to_string()))
            .map(|(_, server)| Arc::clone(server));

        debug!(found = result.is_some(), "IP lookup result");
        result
    }

    pub async fn update_configurations(&self, configs: Vec<ServerConfig>) {
        let span = debug_span!(
            "config_service: update_config_store",
            config_count = configs.len()
        );

        async {
            let mut config_lock = self.configurations.write().await;
            for config in configs {
                debug!(
                    config_id = %config.config_id,
                    domains = ?config.domains,
                    "Updating configuration"
                );
                TELEMETRY.update_backend_count(1, &config.config_id);
                config_lock.insert(config.config_id.clone(), Arc::new(config));
            }
        }
        .instrument(span)
        .await;
    }

    #[instrument(skip(self), fields(config_id = %config_id))]
    pub async fn remove_configuration(&self, config_id: &str) {
        let mut config_lock = self.configurations.write().await;
        debug!(
            config_id = %config_id,
            "Removing configuration"
        );

        TELEMETRY.update_backend_count(-1, &config_id);
        if config_lock.remove(config_id).is_some() {
            debug!("Configuration removed successfully");
        } else {
            debug!("Configuration not found for removal");
        }
    }
}
