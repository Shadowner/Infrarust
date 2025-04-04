use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, debug_span, info, instrument};
use wildmatch::WildMatch;

use crate::core::config::ServerConfig;

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

#[derive(Clone, Debug)]
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

        for config in configs.values() {
            if config
                .domains
                .iter()
                .any(|pattern| WildMatch::new(pattern).matches(&domain))
            {
                debug!(found = true, "Domain lookup result");
                return Some(Arc::clone(config));
            }
        }

        debug!(found = false, "Domain lookup result");
        None
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

    /// Get all configurations
    pub async fn get_all_configurations(&self) -> HashMap<String, Arc<ServerConfig>> {
        let configs = self.configurations.read().await;
        configs.clone()
    }

    pub async fn update_configurations(&self, configs: Vec<ServerConfig>) {
        let span = debug_span!(
            "config_service: update_config_store",
            config_count = configs.len()
        );

        async {
            if configs.is_empty() {
                return;
            }

            let mut added_configs = Vec::new();
            let mut updated_configs = Vec::new();

            {
                let existing_configs = self.configurations.read().await;

                for config in &configs {
                    let config_id = &config.config_id;
                    if existing_configs.contains_key(config_id) {
                        updated_configs.push(config_id.clone());
                    } else {
                        added_configs.push(config_id.clone());
                    }
                }
            }

            {
                let mut config_lock = self.configurations.write().await;

                // Add new configurations and update telemetry
                for config in configs {
                    let config_id = config.config_id.clone();
                    let is_new = !config_lock.contains_key(&config_id);

                    if is_new {
                        #[cfg(feature = "telemetry")]
                        TELEMETRY.update_backend_count(1, &config_id);
                    }

                    config_lock.insert(config_id, Arc::new(config));
                }
            }

            if !added_configs.is_empty() {
                info!(
                    "Added {} new server configurations: {:?}",
                    added_configs.len(),
                    added_configs
                );
            }

            if !updated_configs.is_empty() {
                if updated_configs.len() == 1 {
                    info!("Updated server configuration: {}", updated_configs[0]);
                } else {
                    info!(
                        "Updated {} server configurations: {:?}",
                        updated_configs.len(),
                        updated_configs
                    );
                }
            }
        }
        .instrument(span)
        .await;
    }

    #[instrument(skip(self), fields(config_id = %config_id))]
    pub async fn remove_configuration(&self, config_id: &str) {
        let mut config_lock = self.configurations.write().await;

        info!(
            "Configuration update - Removing server configuration: {}",
            config_id
        );

        debug!(
            config_id = %config_id,
            "Removing configuration"
        );

        #[cfg(feature = "telemetry")]
        TELEMETRY.update_backend_count(-1, config_id);

        if config_lock.remove(config_id).is_some() {
            debug!("Configuration removed successfully");
        } else {
            debug!("Configuration not found for removal");
        }
    }
}
