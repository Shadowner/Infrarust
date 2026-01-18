use infrarust_config::{LogType, ServerConfig};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, debug_span, info, instrument};
use wildmatch::WildMatch;

use crate::server::manager::Manager;

#[derive(Clone, Debug)]
pub struct ConfigurationService {
    configurations: Arc<RwLock<HashMap<String, Arc<ServerConfig>>>>,
    server_managers: Arc<RwLock<Option<Arc<Manager>>>>,
}

impl Default for ConfigurationService {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigurationService {
    #[instrument(name = "create_config_service")]
    pub fn new() -> Self {
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Creating new configuration service"
        );
        Self {
            configurations: Arc::new(RwLock::new(HashMap::new())),
            server_managers: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn set_server_managers(&self, managers: Arc<Manager>) {
        let mut lock = self.server_managers.write().await;
        *lock = Some(managers);
    }

    #[instrument(skip(self), fields(domain = %domain))]
    pub async fn find_server_by_domain(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Finding server by domain"
        );
        let domain = domain.to_lowercase();
        let configs = self.configurations.read().await;

        for config in configs.values() {
            if config
                .domains
                .iter()
                .any(|pattern| WildMatch::new(pattern).matches(&domain))
            {
                debug!(
                    log_type = LogType::ConfigProvider.as_str(),
                    found = true,
                    "Domain lookup result"
                );
                return Some(Arc::clone(config));
            }
        }

        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            found = false,
            "Domain lookup result"
        );
        None
    }

    #[instrument(skip(self), fields(ip = %ip))]
    pub async fn find_server_by_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Finding server by IP"
        );
        let configs = self.configurations.read().await;
        let ip_string = ip.to_string();

        let result = configs
            .values()
            .find(|server| server.addresses.contains(&ip_string))
            .map(Arc::clone);

        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            found = result.is_some(),
            "IP lookup result"
        );
        result
    }

    /// Get all configurations
    pub async fn get_all_configurations(&self) -> HashMap<String, Arc<ServerConfig>> {
        let configs = self.configurations.read().await;
        configs.clone()
    }

    pub fn config_count(&self) -> Option<usize> {
        self.configurations.try_read().ok().map(|c| c.len())
    }

    pub async fn update_configurations(&self, configs: Vec<ServerConfig>) {
        let span = debug_span!(
            "config_service: update_config_store",
            config_count = configs.len(),
            log_type = LogType::ConfigProvider.as_str()
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
                    debug!(
                        log_type = LogType::ConfigProvider.as_str(),
                        "Config ID: {:?}", config
                    );
                    if let Some(manager_config) = &config.server_manager
                        && let Some(local_config_provider) = &manager_config.local_provider
                    {
                        let managers_guard = self.server_managers.read().await;
                        if let Some(managers) = managers_guard.as_ref() {
                            debug!(
                                log_type = LogType::ServerManager.as_str(),
                                "Registering server with ID to the Local Provider {}",
                                manager_config.server_id
                            );
                            managers
                                .local_provider()
                                .api_client()
                                .register_server(
                                    &manager_config.server_id,
                                    local_config_provider.clone(),
                                )
                                .await;
                        }
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
                        {
                            // Placeholder for telemetry integration
                            // Would be implemented when telemetry feature is enabled
                        }
                    }

                    config_lock.insert(config_id, Arc::new(config));
                }
            }

            if !added_configs.is_empty() {
                info!(
                    log_type = LogType::ConfigProvider.as_str(),
                    "Added {} new server configurations: {:?}",
                    added_configs.len(),
                    added_configs
                );
            }

            if !updated_configs.is_empty() {
                if updated_configs.len() == 1 {
                    info!(
                        log_type = LogType::ConfigProvider.as_str(),
                        "Updated server configuration: {}", updated_configs[0]
                    );
                } else {
                    info!(
                        log_type = LogType::ConfigProvider.as_str(),
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
            log_type = LogType::ConfigProvider.as_str(),
            "Configuration update - Removing server configuration: {}", config_id
        );

        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            config_id = %config_id,
            "Removing configuration"
        );

        #[cfg(feature = "telemetry")]
        {
            // Placeholder for telemetry integration
            // Would be implemented when telemetry feature is enabled
        }

        let config = config_lock.get(config_id).cloned();
        if let Some(config) = config
            && let Some(manager_config) = &config.server_manager
        {
            let managers_guard = self.server_managers.read().await;
            if let Some(managers) = managers_guard.as_ref() {
                managers
                    .local_provider()
                    .api_client()
                    .unregister_server(&manager_config.server_id)
                    .await;
            }
        }

        if config_lock.remove(config_id).is_some() {
            debug!(
                log_type = LogType::ConfigProvider.as_str(),
                "Configuration removed successfully"
            );
        } else {
            debug!(
                log_type = LogType::ConfigProvider.as_str(),
                "Configuration not found for removal"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    #[test]
    fn test_file_provider() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");
        let proxies_path = temp_dir.path().join("proxies");

        fs::create_dir(&proxies_path).unwrap();

        fs::write(&config_path, "bind: ':25565'\n").unwrap();
        fs::write(
            proxies_path.join("server1.yml"),
            "domains: ['example.com']\naddresses: ['127.0.0.1:25566']\n",
        )
        .unwrap();

        // let provider = FileProvider::new(
        //     config_path.to_str().unwrap().to_string(),
        //     proxies_path.to_str().unwrap().to_string(),
        //     FileType::Yaml,
        // );

        // let config = provider.load_config().unwrap();
        // assert!(!config.server_configs.is_empty());
    }
}
