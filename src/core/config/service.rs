use log::debug;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use wildmatch::WildMatch;

use crate::core::config::ServerConfig;

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
    pub fn new() -> Self {
        Self {
            configurations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn find_server_by_domain(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!("Finding server by domain: {}", domain);
        let domain = domain.to_lowercase();
        let configs = self.configurations.read().await;
        configs
            .values()
            .find(|server| {
                server
                    .domains
                    .iter()
                    .any(|pattern| WildMatch::new(pattern).matches(&domain))
            })
            .cloned()
    }

    pub async fn find_server_by_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        debug!("Finding server by ip: {}", ip);
        let configs = self.configurations.read().await;
        configs
            .iter()
            .find(|(_, server)| server.addresses.contains(&ip.to_string()))
            .map(|(_, server)| Arc::clone(server))
    }

    pub async fn update_configurations(&self, configs: Vec<ServerConfig>) {
        debug!("Updating configurations with length: {}", configs.len());
        let mut config_lock = self.configurations.write().await;
        for config in configs {
            config_lock.insert(config.config_id.clone(), Arc::new(config));
        }
    }

    pub async fn remove_configuration(&self, config_id: &str) {
        debug!("Removing configuration with id: {}", config_id);
        let mut config_lock = self.configurations.write().await;
        config_lock.remove(config_id);
    }
}
