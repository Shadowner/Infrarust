use std::collections::HashMap;

use bollard::{container::ListContainersOptions, secret::ContainerSummary};
use tracing::{debug, error, instrument};

use crate::models::server::{ProxyModeEnum, ServerConfig};

use super::DockerProvider;

impl DockerProvider {
    #[instrument(skip(self), name = "docker_provider: load_containers")]
    pub(crate) async fn load_containers(
        &self,
    ) -> Result<HashMap<String, ServerConfig>, bollard::errors::Error> {
        let docker = self.docker.as_ref().expect("Docker client not initialized");

        let mut configs = HashMap::new();
        let containers = docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters: HashMap::from([("status".to_string(), vec!["running".to_string()])]),
                ..Default::default()
            }))
            .await?;

        debug!(
            log_type = "config_provider",
            "Found {} running containers",
            containers.len()
        );

        for container in containers {
            if let Some(config) = self.process_container(&container).await {
                let id = container.id.as_deref().unwrap_or("unknown").to_string();
                configs.insert(self.generate_config_id(&id), config);

                let mut tracked = self.tracked_containers.write().await;
                tracked.insert(id);
            }
        }

        Ok(configs)
    }

    #[instrument(skip(self, container), name = "docker_provider: process_container")]
    pub(crate) async fn process_container(
        &self,
        container: &ContainerSummary,
    ) -> Option<ServerConfig> {
        let container_id = container.id.as_deref()?;
        let container_name = container.names.as_ref()?.first()?.trim_start_matches('/');

        debug!(container_id = %container_id, name = %container_name, "Processing container");

        let labels = container.labels.as_ref()?;

        if !labels
            .keys()
            .any(|k| k.starts_with(&format!("{}.enable", self.config.label_prefix)))
        {
            debug!(container_id = %container_id, "Skipping container without Infrarust labels");
            return None;
        }

        let mut domains = Vec::new();
        if let Some(domain_str) = labels.get(&format!("{}.domains", self.config.label_prefix)) {
            domains = domain_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
        }

        if domains.is_empty() {
            if self.config.default_domains.is_empty() {
                domains.push(format!("{}.docker.local", container_name));
            } else {
                for domain in &self.config.default_domains {
                    domains.push(format!("{}.{}", container_name, domain));
                }
            }
        }

        let mut addresses = Vec::new();
        if let Some(custom_address) = labels.get(&format!("{}.address", self.config.label_prefix)) {
            addresses.push(custom_address.clone());
        }

        let target_port = labels
            .get(&format!("{}.port", self.config.label_prefix))
            .map(|p| p.parse::<u16>().unwrap_or(25565))
            .unwrap_or(25565);

        let mut found_container_ips = false;
        if let Some(networks) = &container.network_settings.as_ref()?.networks {
            for (network_name, network) in networks {
                if let Some(ip) = &network.ip_address
                    && !ip.is_empty()
                    && ip != "0.0.0.0"
                {
                    debug!(
                        container_id = %container_id,
                        network = %network_name,
                        ip = %ip,
                        port = %target_port,
                        "Using container network IP address"
                    );
                    addresses.push(format!("{}:{}", ip, target_port));
                    found_container_ips = true;
                }
            }
        }

        if !found_container_ips
            && self.docker.is_some()
            && let Some(docker) = &self.docker
        {
            match docker.inspect_container(container_id, None).await {
                Ok(container_info) => {
                    if let Some(network_settings) = container_info.network_settings
                        && let Some(networks) = network_settings.networks
                    {
                        for (network_name, network) in networks {
                            if let Some(ip) = network.ip_address
                                && !ip.is_empty()
                                && ip != "0.0.0.0"
                            {
                                debug!(
                                    container_id = %container_id,
                                    network = %network_name,
                                    ip = %ip,
                                    port = %target_port,
                                    "Using container IP from detailed inspection"
                                );
                                addresses.push(format!("{}:{}", ip, target_port));
                                found_container_ips = true;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to inspect container for network info {}: {}",
                        container_id, e
                    );
                }
            }
        }

        // If we still don't have any container IPs, fall back to port mappings
        if !found_container_ips {
            debug!(container_id = %container_id, "No container network IPs found, falling back to port mappings");
            let port_bindings = container
                .ports
                .as_ref()?
                .iter()
                .filter_map(|port| {
                    let public_port = port.public_port?;
                    let private_port = port.private_port;
                    if private_port == target_port {
                        let host_ip = port.ip.as_deref().unwrap_or("0.0.0.0");
                        let actual_ip = if host_ip == "0.0.0.0" {
                            "127.0.0.1"
                        } else {
                            host_ip
                        };

                        Some(format!("{}:{}", actual_ip, public_port))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            addresses.extend(port_bindings);
        }

        if addresses.is_empty() {
            debug!(
                container_id = %container_id,
                container_name = %container_name,
                "Using container name as hostname with default port"
            );
            addresses.push(format!("{}:{}", container_name, target_port));
        }

        if addresses.is_empty() {
            debug!(container_id = %container_id, "No usable addresses found, skipping container");
            return None;
        }

        let proxy_mode_str = labels
            .get(&format!("{}.proxy_mode", self.config.label_prefix))
            .map(|s| s.as_str());
        let proxy_mode = match proxy_mode_str {
            Some("passthrough") => Some(ProxyModeEnum::Passthrough),
            Some("offline") => Some(ProxyModeEnum::Offline),
            Some("server_only") => Some(ProxyModeEnum::ServerOnly),
            Some("client_only") => Some(ProxyModeEnum::ClientOnly),
            _ => None, // Default
        };

        let send_proxy_protocol = labels
            .get(&format!("{}.proxy_protocol", self.config.label_prefix))
            .map(|v| v.to_lowercase() == "true" || v == "1");

        let server_config = ServerConfig {
            domains,
            addresses,
            send_proxy_protocol,
            proxy_mode,
            config_id: self.generate_config_id(container_id),
            ..Default::default()
        };

        debug!(container_id = %container_id, "Created server config: {:?}", server_config);
        Some(server_config)
    }

    pub(crate) fn generate_config_id(&self, container_id: &str) -> String {
        format!("docker@{}", container_id)
    }
}
