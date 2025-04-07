use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use bollard::{
    Docker,
    container::ListContainersOptions,
    models::{ContainerStateStatusEnum, EventMessage},
    secret::ContainerSummary,
    system::EventsOptions,
};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::{
    RwLock,
    mpsc::{self, Sender},
};
use tracing::{Instrument, debug, debug_span, error, info, instrument, warn};

use crate::core::{
    config::{ServerConfig, provider::Provider},
    event::ProviderMessage,
};

/// Configuration for the Docker provider
#[derive(Debug, Clone, Deserialize)]
pub struct DockerProviderConfig {
    #[serde(default)]
    pub docker_host: String,

    #[serde(default)]
    pub label_prefix: String,
    #[serde(default)]
    pub polling_interval: u64,
    #[serde(default)]
    pub watch: bool,
    #[serde(default)]
    pub default_domains: Vec<String>,
}

impl Default for DockerProviderConfig {
    fn default() -> Self {
        Self {
            docker_host: "unix:///var/run/docker.sock".to_string(),
            label_prefix: "infrarust".to_string(),
            polling_interval: 10,
            watch: true,
            default_domains: Vec::new(),
        }
    }
}

pub struct DockerProvider {
    config: DockerProviderConfig,
    docker: Option<Docker>,
    sender: Sender<ProviderMessage>,
    tracked_containers: Arc<RwLock<HashSet<String>>>,
    previous_configs: Arc<RwLock<HashMap<String, ServerConfig>>>,
}

impl DockerProvider {
    #[instrument(skip(sender), fields(docker_host = %config.docker_host), name = "docker_provider: new")]
    pub fn new(config: DockerProviderConfig, sender: Sender<ProviderMessage>) -> Self {
        debug!("Initializing Docker provider with config: {:?}", config);
        Self {
            config,
            docker: None,
            sender,
            tracked_containers: Arc::new(RwLock::new(HashSet::new())),
            previous_configs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[instrument(skip(self), name = "docker_provider: connect")]
    async fn connect(&mut self) -> Result<(), bollard::errors::Error> {
        debug!("Connecting to Docker daemon: {}", self.config.docker_host);

        let docker = if self.config.docker_host.starts_with("unix://") {
            Docker::connect_with_socket_defaults()?
        } else if self.config.docker_host.starts_with("tcp://") {
            Docker::connect_with_http_defaults()?
        } else {
            Docker::connect_with_local_defaults()?
        };

        docker.ping().await?;
        info!("Successfully connected to Docker daemon");

        self.docker = Some(docker);
        Ok(())
    }

    #[instrument(skip(self), name = "docker_provider: load_containers")]
    async fn load_containers(
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

        debug!("Found {} running containers", containers.len());

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
    async fn process_container(&self, container: &ContainerSummary) -> Option<ServerConfig> {
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
                if let Some(ip) = &network.ip_address {
                    if !ip.is_empty() && ip != "0.0.0.0" {
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
        }

        if !found_container_ips && self.docker.is_some() {
            if let Some(docker) = &self.docker {
                match docker.inspect_container(container_id, None).await {
                    Ok(container_info) => {
                        if let Some(network_settings) = container_info.network_settings {
                            if let Some(networks) = network_settings.networks {
                                for (network_name, network) in networks {
                                    if let Some(ip) = network.ip_address {
                                        if !ip.is_empty() && ip != "0.0.0.0" {
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
            Some("passthrough") => Some(crate::proxy_modes::ProxyModeEnum::Passthrough),
            Some("offline") => Some(crate::proxy_modes::ProxyModeEnum::Offline),
            Some("server_only") => Some(crate::proxy_modes::ProxyModeEnum::ServerOnly),
            Some("client_only") => Some(crate::proxy_modes::ProxyModeEnum::ClientOnly),
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

    fn generate_config_id(&self, container_id: &str) -> String {
        format!("docker@{}", container_id)
    }

    #[instrument(skip(self), name = "docker_provider: watch_events")]
    async fn watch_events(&self) -> Result<(), bollard::errors::Error> {
        let docker = self.docker.as_ref().expect("Docker client not initialized");

        let mut options = EventsOptions::<String>::default();

        options
            .filters
            .insert("type".to_string(), vec!["container".to_string()]);
        options.filters.insert(
            "event".to_string(),
            vec![
                "start".to_string(),
                "stop".to_string(),
                "die".to_string(),
                "kill".to_string(),
                "destroy".to_string(),
                "create".to_string(),
            ],
        );

        let mut event_stream = docker.events(Some(options));
        info!("Watching Docker events for container lifecycle changes");

        while let Some(event) = event_stream.next().await {
            match event {
                Ok(event) => {
                    let action = event.action.as_deref().unwrap_or("");

                    // TODO: Might be unecessary now
                    let is_relevant = matches!(
                        action,
                        "start" | "stop" | "die" | "kill" | "destroy" | "create"
                    );

                    if is_relevant {
                        self.handle_docker_event(event).await;
                    }
                }
                Err(e) => {
                    error!("Error watching Docker events: {}", e);
                    return Err(e);
                }
            }
        }

        warn!("Docker event stream ended");
        Ok(())
    }

    #[instrument(
        skip(self, event),
        fields(
            action = %event.action.as_deref().unwrap_or("unknown"),
            container_id = %event.actor.as_ref().and_then(|a| a.id.as_deref()).unwrap_or("unknown")
        ),
        level = "debug",
        name = "docker_provider: handle_event"
    )]
    async fn handle_docker_event(&self, event: EventMessage) {
        let container_id = match event.actor.as_ref().and_then(|a| a.id.as_ref()) {
            Some(id) => id,
            None => return,
        };

        let action = event.action.as_deref().unwrap_or("unknown");

        debug!(container_id = %container_id, action = %action, "Processing container lifecycle event");

        match action {
            "start" => {
                if let Some(docker) = &self.docker {
                    match docker.inspect_container(container_id, None).await {
                        Ok(container_info) => {
                            if container_info.state.and_then(|s| s.status)
                                == Some(ContainerStateStatusEnum::RUNNING)
                            {
                                let options = ListContainersOptions {
                                    all: false,
                                    filters: HashMap::from([(
                                        "id".to_string(),
                                        vec![container_id.to_string()],
                                    )]),
                                    ..Default::default()
                                };

                                if let Ok(containers) = docker.list_containers(Some(options)).await
                                {
                                    if let Some(container) = containers.first() {
                                        if let Some(config) =
                                            self.process_container(container).await
                                        {
                                            let key = self.generate_config_id(container_id);
                                            self.send_update(key, Some(config)).await;

                                            let mut tracked = self.tracked_containers.write().await;
                                            tracked.insert(container_id.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => error!("Failed to inspect container {}: {}", container_id, e),
                    }
                }
            }
            "die" | "stop" | "kill" | "destroy" => {
                let key = self.generate_config_id(container_id);
                let contains_id = {
                    let tracked = self.tracked_containers.read().await;
                    tracked.contains(container_id)
                };

                if contains_id {
                    self.send_update(key, None).await;
                    let mut tracked = self.tracked_containers.write().await;
                    tracked.remove(container_id);
                }
            }
            _ => {
                // We shouldn't get here with our filtered events, but just in case
                debug!(container_id = %container_id, action = %action, "Ignoring irrelevant container event");
            }
        }
    }

    #[instrument(skip(self, config), fields(key = %key), name = "docker_provider: send_update")]
    async fn send_update(&self, key: String, config: Option<ServerConfig>) {
        let span =
            debug_span!("docker_provider: send_update", key = %key, has_config = config.is_some());

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
            debug!("Skipping update for {} (no changes)", key);
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
            debug!("Sending config update for {}", key);
            if let Err(e) = self
                .sender
                .send(ProviderMessage::Update {
                    key,
                    configuration: Some(Box::new(config)),
                    span: span.clone(),
                })
                .await
            {
                error!("Failed to send container update: {}", e);
            }
        } else {
            debug!("Removing config for {}", key);
            if let Err(e) = self
                .sender
                .send(ProviderMessage::Update {
                    key,
                    configuration: None,
                    span: span.clone(),
                })
                .await
            {
                error!("Failed to send container removal: {}", e);
            }
        }
    }

    /// TODO: Might move it in a separate module (mod config ?)
    fn configs_are_equal(&self, a: &ServerConfig, b: &ServerConfig) -> bool {
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

#[async_trait]
impl Provider for DockerProvider {
    #[instrument(skip(self), name = "docker_provider: run")]
    async fn run(&mut self) {
        let span = debug_span!("docker_provider_run");

        async {
            if let Err(e) = self.connect().await {
                error!("Failed to connect to Docker daemon: {}", e);
                return;
            }

            match self.load_containers().await {
                Ok(configs) => {
                    info!("Loaded {} container configurations", configs.len());

                    // Send initial configurations
                    let mut server_configs = HashMap::new();
                    for (key, config) in configs {
                        server_configs.insert(key, config);
                    }

                    if !server_configs.is_empty() {
                        if let Err(e) = self
                            .sender
                            .send(ProviderMessage::FirstInit(server_configs))
                            .await
                        {
                            error!("Failed to send initial configurations: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to load containers: {}", e);
                }
            }

            if self.config.watch {
                let docker_clone = self.docker.clone();
                let tracked_containers = self.tracked_containers.clone();
                let config = self.config.clone();
                let sender = self.sender.clone();

                let event_span = debug_span!("docker_event_watcher");
                tokio::spawn(
                    async move {
                        let event_provider = DockerProvider {
                            config: config.clone(),
                            docker: docker_clone.clone(),
                            sender: sender.clone(),
                            tracked_containers: tracked_containers.clone(),
                            previous_configs: Arc::new(RwLock::new(HashMap::new())),
                        };

                        if let Err(e) = event_provider.watch_events().await {
                            error!("Docker event watcher failed: {}", e);
                        }
                    }
                    .instrument(event_span),
                );

                // OLD : Polling method
                // let polling_interval = self.config.polling_interval;
                // if polling_interval > 0 {
                //     let polling_span = debug_span!("docker_polling");

                //     let docker_clone = self.docker.clone();
                //     let tracked_containers = self.tracked_containers.clone();
                //     let config = self.config.clone();
                //     let sender = self.sender.clone();

                //     tokio::spawn(
                //         async move {
                //             let polling_provider = DockerProvider {
                //                 config: config.clone(),
                //                 docker: docker_clone,
                //                 sender,
                //                 tracked_containers: tracked_containers.clone(),
                //                 previous_configs: Arc::new(RwLock::new(HashMap::new())),
                //             };

                //             let mut interval =
                //                 tokio::time::interval(Duration::from_secs(polling_interval));
                //             loop {
                //                 interval.tick().await;

                //                 debug!("Polling for container changes");
                //                 match polling_provider.load_containers().await {
                //                     Ok(new_configs) => {
                //                         let current_ids: HashSet<String> = new_configs
                //                             .keys()
                //                             .map(|k| k.split('@').nth(1).unwrap_or("").to_string())
                //                             .collect();

                //                         let tracked = tracked_containers.read().await;
                //                         let removed_ids: Vec<String> = tracked
                //                             .iter()
                //                             .filter(|id| !current_ids.contains(*id))
                //                             .cloned()
                //                             .collect();
                //                         drop(tracked);

                //                         for id in &removed_ids {
                //                             let key = polling_provider.generate_config_id(id);
                //                             polling_provider.send_update(key, None).await;
                //                         }

                //                         if !removed_ids.is_empty() {
                //                             let mut tracked = tracked_containers.write().await;
                //                             for id in &removed_ids {
                //                                 tracked.remove(id);
                //                             }
                //                         }

                //                         for (key, config) in new_configs {
                //                             polling_provider.send_update(key, Some(config)).await;
                //                         }
                //                     }
                //                     Err(e) => {
                //                         error!("Failed to poll containers: {}", e);
                //                     }
                //                 }
                //             }
                //         }
                //         .instrument(polling_span),
                //     );
                // }
            }

            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                debug!("Docker provider heartbeat");
            }
        }
        .instrument(span)
        .await
    }

    fn get_name(&self) -> String {
        "DockerProvider".to_string()
    }

    fn new(sender: mpsc::Sender<ProviderMessage>) -> Self {
        Self {
            config: DockerProviderConfig::default(),
            docker: None,
            sender,
            tracked_containers: Arc::new(RwLock::new(HashSet::new())),
            previous_configs: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Clone for DockerProvider {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            docker: self.docker.clone(),
            sender: self.sender.clone(),
            tracked_containers: self.tracked_containers.clone(),
            previous_configs: self.previous_configs.clone(),
        }
    }
}
