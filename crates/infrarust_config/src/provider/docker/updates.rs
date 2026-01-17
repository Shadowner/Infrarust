use std::collections::HashSet;

use tracing::{debug, debug_span, error, instrument};

use crate::{models::server::ServerConfig, provider::ProviderMessage};

use super::DockerProvider;

impl DockerProvider {
    #[instrument(skip(self, config), fields(key = %key), name = "docker_provider: send_update")]
    pub(crate) async fn send_update(&self, key: String, config: Option<ServerConfig>) {
        let span = debug_span!("docker_provider: send_update", key = %key, has_config = config.is_some());

        let should_send = match &config {
            Some(new_config) => {
                let prev_configs = self.previous_configs.read().await;
                match prev_configs.get(&key) {
                    Some(prev_config) => !self.configs_are_equal(prev_config, new_config),
                    None => true,
                }
            }
            None => {
                let prev_configs = self.previous_configs.read().await;
                prev_configs.contains_key(&key)
            }
        };

        if !should_send {
            debug!(
                log_type = "config_provider",
                "Skipping update for {} (no changes)", key
            );
            return;
        }

        {
            let mut prev_configs = self.previous_configs.write().await;
            match &config {
                Some(cfg) => {
                    prev_configs.insert(key.clone(), cfg.clone());
                }
                None => {
                    prev_configs.remove(&key);
                }
            }
        }

        if let Some(config) = config {
            debug!(
                log_type = "config_provider",
                "Sending config update for {}", key
            );
            if let Err(e) = self
                .sender
                .send(ProviderMessage::Update {
                    key,
                    configuration: Some(Box::new(config)),
                    span: span.clone(),
                })
                .await
            {
                error!(
                    log_type = "config_provider",
                    "Failed to send container update: {}", e
                );
            }
        } else {
            debug!(log_type = "config_provider", "Removing config for {}", key);
            if let Err(e) = self
                .sender
                .send(ProviderMessage::Update {
                    key,
                    configuration: None,
                    span: span.clone(),
                })
                .await
            {
                error!(
                    log_type = "config_provider",
                    "Failed to send container removal: {}", e
                );
            }
        }
    }

    pub(crate) fn configs_are_equal(&self, a: &ServerConfig, b: &ServerConfig) -> bool {
        if a.domains != b.domains {
            return false;
        }

        let a_addrs: HashSet<_> = a.addresses.iter().collect();
        let b_addrs: HashSet<_> = b.addresses.iter().collect();
        if a_addrs != b_addrs {
            return false;
        }

        if a.send_proxy_protocol != b.send_proxy_protocol
            || a.proxy_protocol_version != b.proxy_protocol_version
        {
            return false;
        }

        if a.proxy_mode != b.proxy_mode {
            return false;
        }

        if (a.filters.is_some() && b.filters.is_none())
            || (a.filters.is_none() && b.filters.is_some())
        {
            return false;
        }

        true
    }
}
