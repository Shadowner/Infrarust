use infrarust_config::{LogType, ServerConfig};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::{Instrument, debug, debug_span, info, instrument};
use wildmatch::WildMatch;

use crate::server::gateway::Gateway;

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
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Creating new configuration service"
        );
        Self {
            configurations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[instrument(skip(self), fields(domain = %domain))]
    pub async fn find_server_by_domain(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Finding server by domain"
        );
        let domain = domain.to_lowercase();
        let configs_snapshot = {
            let configs = self.configurations.read().await;
            configs.clone()
        };

        // Track the best (highest specificity) match per config.
        // Uses a HashMap keyed by config_id to ensure each config appears at most once,
        // storing only the highest specificity score when multiple patterns match.
        let mut best_matches: HashMap<String, (Arc<ServerConfig>, i32)> = HashMap::new();

        for config in configs_snapshot.values() {
            for pattern in &config.domains {
                if WildMatch::new(pattern).matches(&domain) {
                    let specificity = Self::calculate_pattern_specificity(pattern);

                    // Only keep this match if it's the first or has higher specificity
                    best_matches
                        .entry(config.config_id.clone())
                        .and_modify(|(_, existing_score)| {
                            if specificity > *existing_score {
                                *existing_score = specificity;
                            }
                        })
                        .or_insert_with(|| (Arc::clone(config), specificity));
                }
            }
        }

        if best_matches.is_empty() {
            debug!(
                log_type = LogType::ConfigProvider.as_str(),
                found = false,
                "Domain lookup result"
            );
            return None;
        }

        // Find the config with the highest specificity score
        let best_match = best_matches
            .values()
            .max_by_key(|(_, specificity)| specificity)
            .map(|(config, _)| Arc::clone(config));
        
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            found = true,
            "Domain lookup result"
        );
        
        best_match
    }
    
    /// Calculates the domain specificity score for sorting.
    /// Higher scores indicate more specific patterns, which should be checked first.
    fn calculate_pattern_specificity(pattern: &str) -> i32 {
        let pattern_lower = pattern.to_lowercase();
        
        // Exact matches (no wildcards) get highest priority
        if !pattern_lower.contains('*') && !pattern_lower.contains('?') {
            return 10000;
        }
        
        // For wildcard patterns, count the number of non-wildcard segments
        // More segments = more specific
        let segments: Vec<&str> = pattern_lower.split('.').collect();
        let mut score = 0;
        
        // Count non-wildcard segments
        for segment in &segments {
            if !segment.contains('*') && !segment.contains('?') {
                score += 100;
            }
        }
        
        // Add bonus for total number of segments (more dots = more specific)
        score += segments.len() as i32;
        
        score
    }

    #[instrument(skip(self), fields(ip = %ip))]
    pub async fn find_server_by_ip(&self, ip: &str) -> Option<Arc<ServerConfig>> {
        debug!(
            log_type = LogType::ConfigProvider.as_str(),
            "Finding server by IP"
        );
        let configs_snapshot = {
            let configs = self.configurations.read().await;
            configs.clone()
        };
        let result = configs_snapshot
            .iter()
            .find(|(_, server)| server.addresses.contains(&ip.to_string()))
            .map(|(_, server)| Arc::clone(server));

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
                        && let Some(shared) = Gateway::get_shared_component()
                    {
                        debug!(
                            log_type = LogType::ServerManager.as_str(),
                            "Registering server with ID to the Local Provider {}",
                            manager_config.server_id
                        );
                        shared
                            .server_managers()
                            .local_provider()
                            .api_client()
                            .register_server(
                                &manager_config.server_id,
                                local_config_provider.clone(),
                            );
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
            && let Some(shared) = Gateway::get_shared_component()
        {
            shared
                .server_managers()
                .local_provider()
                .api_client()
                .unregister_server(&manager_config.server_id);
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
    use super::*;
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

    #[tokio::test]
    async fn test_wildcard_domain_priority() {
        let service = ConfigurationService::new();

        // Create configurations with different specificity levels
        let configs = vec![
            ServerConfig {
                config_id: "wildcard-base".to_string(),
                domains: vec!["*.example.com".to_string()],
                addresses: vec!["127.0.0.1:25566".to_string()],
                ..Default::default()
            },
            ServerConfig {
                config_id: "wildcard-subdomain".to_string(),
                domains: vec!["*.sub.example.com".to_string()],
                addresses: vec!["127.0.0.1:25567".to_string()],
                ..Default::default()
            },
            ServerConfig {
                config_id: "exact-match".to_string(),
                domains: vec!["test.sub.example.com".to_string()],
                addresses: vec!["127.0.0.1:25568".to_string()],
                ..Default::default()
            },
        ];

        service.update_configurations(configs).await;

        // Test 1: Exact match should be found first
        let result = service.find_server_by_domain("test.sub.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "exact-match");

        // Test 2: More specific wildcard should match before less specific
        let result = service.find_server_by_domain("other.sub.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "wildcard-subdomain");

        // Test 3: Less specific wildcard should match when more specific doesn't
        let result = service.find_server_by_domain("something.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "wildcard-base");
    }

    #[tokio::test]
    async fn test_multiple_wildcards_same_level() {
        let service = ConfigurationService::new();

        let configs = vec![
            ServerConfig {
                config_id: "wildcard-1".to_string(),
                domains: vec!["*.server1.example.com".to_string()],
                addresses: vec!["127.0.0.1:25566".to_string()],
                ..Default::default()
            },
            ServerConfig {
                config_id: "wildcard-2".to_string(),
                domains: vec!["*.server2.example.com".to_string()],
                addresses: vec!["127.0.0.1:25567".to_string()],
                ..Default::default()
            },
        ];

        service.update_configurations(configs).await;

        let result1 = service
            .find_server_by_domain("test.server1.example.com")
            .await;
        assert!(result1.is_some());
        assert_eq!(result1.unwrap().config_id, "wildcard-1");

        let result2 = service
            .find_server_by_domain("test.server2.example.com")
            .await;
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().config_id, "wildcard-2");
    }

    #[tokio::test]
    async fn test_deeply_nested_wildcards() {
        let service = ConfigurationService::new();

        let configs = vec![
            ServerConfig {
                config_id: "level-1".to_string(),
                domains: vec!["*.example.com".to_string()],
                addresses: vec!["127.0.0.1:25566".to_string()],
                ..Default::default()
            },
            ServerConfig {
                config_id: "level-2".to_string(),
                domains: vec!["*.sub.example.com".to_string()],
                addresses: vec!["127.0.0.1:25567".to_string()],
                ..Default::default()
            },
            ServerConfig {
                config_id: "level-3".to_string(),
                domains: vec!["*.deep.sub.example.com".to_string()],
                addresses: vec!["127.0.0.1:25568".to_string()],
                ..Default::default()
            },
        ];

        service.update_configurations(configs).await;

        // Most specific should match first
        let result = service
            .find_server_by_domain("test.deep.sub.example.com")
            .await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "level-3");

        // Medium specificity
        let result = service.find_server_by_domain("test.sub.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "level-2");

        // Least specific
        let result = service.find_server_by_domain("test.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().config_id, "level-1");
    }
}
