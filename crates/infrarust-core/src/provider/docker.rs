//! Docker-based configuration provider.
//!
//! Discovers Minecraft servers from Docker containers with `infrarust.*`
//! labels and watches Docker events for real-time updates.
//!
//! Entirely feature-gated behind `docker`.

use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;

use bollard::Docker;
use bollard::container::{InspectContainerOptions, ListContainersOptions};
use bollard::models::ContainerInspectResponse;
use bollard::system::EventsOptions;
use futures_util::StreamExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use infrarust_config::{DockerProviderConfig, ProxyMode, ServerAddress, ServerConfig};

use crate::error::CoreError;
use crate::provider::{ConfigProvider, ProviderConfig, ProviderEvent, ProviderId};

/// Default Minecraft port.
const DEFAULT_MC_PORT: u16 = 25565;

/// Docker provider that auto-discovers containers with `infrarust.*` labels.
pub struct DockerProvider {
    config: DockerProviderConfig,
    /// Known containers: container_name → ServerConfig
    known: Mutex<HashMap<String, ServerConfig>>,
}

impl DockerProvider {
    /// Creates a new Docker provider from the given configuration.
    pub fn new(config: &DockerProviderConfig) -> Result<Self, CoreError> {
        Ok(Self {
            config: config.clone(),
            known: Mutex::new(HashMap::new()),
        })
    }

    /// Connects to the Docker daemon.
    fn connect(&self) -> Result<Docker, CoreError> {
        let docker = if self.config.endpoint.starts_with("unix://") {
            Docker::connect_with_socket_defaults()
        } else if self.config.endpoint.starts_with("tcp://") {
            Docker::connect_with_http_defaults()
        } else {
            Docker::connect_with_local_defaults()
        }
        .map_err(|e| CoreError::DockerConnection(e.to_string()))?;

        Ok(docker)
    }

    /// Scans all running containers with `infrarust.enable=true`.
    async fn scan_containers(&self, docker: &Docker) -> Result<Vec<ProviderConfig>, CoreError> {
        let mut filters = HashMap::new();
        filters.insert("label", vec!["infrarust.enable=true"]);
        filters.insert("status", vec!["running"]);

        let options = ListContainersOptions {
            all: false,
            filters,
            ..Default::default()
        };

        let containers = docker
            .list_containers(Some(options))
            .await
            .map_err(|e| CoreError::DockerConnection(e.to_string()))?;

        let mut configs = Vec::new();

        for container in &containers {
            let container_id = match &container.id {
                Some(id) => id.as_str(),
                None => continue,
            };

            let container_name = container
                .names
                .as_ref()
                .and_then(|names| names.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| container_id[..12].to_string());

            match self
                .inspect_and_build(docker, container_id, &container_name)
                .await
            {
                Ok(Some(pc)) => configs.push(pc),
                Ok(None) => {} // Not enabled or no labels
                Err(e) => {
                    tracing::warn!(
                        container = %container_name,
                        error = %e,
                        "failed to process container, skipping"
                    );
                }
            }
        }

        Ok(configs)
    }

    /// Inspects a container and builds a ProviderConfig if it has infrarust labels.
    async fn inspect_and_build(
        &self,
        docker: &Docker,
        container_id: &str,
        container_name: &str,
    ) -> Result<Option<ProviderConfig>, CoreError> {
        let info = docker
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| CoreError::DockerConnection(e.to_string()))?;

        let labels = match info.config.as_ref().and_then(|c| c.labels.as_ref()) {
            Some(labels) => labels,
            None => return Ok(None),
        };

        if labels.get("infrarust.enable").map(|v| v.as_str()) != Some("true") {
            return Ok(None);
        }

        let port = labels
            .get("infrarust.port")
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_MC_PORT);

        let address = resolve_container_address(&info, self.config.network.as_deref(), port);

        let config = labels_to_server_config(container_name, labels, &address);

        Ok(Some(ProviderConfig {
            id: ProviderId::docker(container_name),
            config,
        }))
    }

    /// Watches Docker events with automatic reconnection.
    async fn watch_with_reconnect(
        &self,
        sender: &mpsc::Sender<ProviderEvent>,
        shutdown: &CancellationToken,
    ) -> Result<(), CoreError> {
        let mut reconnect_delay = self.config.reconnect_delay;
        let max_delay = Duration::from_secs(60);

        loop {
            let docker = match self.connect() {
                Ok(d) => d,
                Err(e) => {
                    if shutdown.is_cancelled() {
                        return Ok(());
                    }
                    tracing::warn!(
                        error = %e,
                        delay = ?reconnect_delay,
                        "failed to connect to docker, retrying"
                    );
                    tokio::select! {
                        biased;
                        () = shutdown.cancelled() => return Ok(()),
                        () = tokio::time::sleep(reconnect_delay) => {}
                    }
                    reconnect_delay = (reconnect_delay * 2).min(max_delay);
                    continue;
                }
            };

            // Reset delay on successful connection
            reconnect_delay = self.config.reconnect_delay;

            match self.watch_events(&docker, sender, shutdown).await {
                Ok(()) => return Ok(()), // Normal shutdown
                Err(e) => {
                    if shutdown.is_cancelled() {
                        return Ok(());
                    }
                    tracing::warn!(
                        error = %e,
                        delay = ?reconnect_delay,
                        "docker event stream disconnected, reconnecting"
                    );
                    tokio::select! {
                        biased;
                        () = shutdown.cancelled() => return Ok(()),
                        () = tokio::time::sleep(reconnect_delay) => {}
                    }
                    reconnect_delay = (reconnect_delay * 2).min(max_delay);

                    // Re-scan after reconnection
                    if let Ok(docker) = self.connect() {
                        self.resync_containers(&docker, sender).await;
                    }
                }
            }
        }
    }

    /// Watches the Docker event stream.
    async fn watch_events(
        &self,
        docker: &Docker,
        sender: &mpsc::Sender<ProviderEvent>,
        shutdown: &CancellationToken,
    ) -> Result<(), CoreError> {
        let mut filters = HashMap::new();
        filters.insert("type".to_string(), vec!["container".to_string()]);
        filters.insert(
            "event".to_string(),
            vec![
                "start".to_string(),
                "stop".to_string(),
                "die".to_string(),
                "destroy".to_string(),
                "pause".to_string(),
                "unpause".to_string(),
            ],
        );

        let options = EventsOptions {
            filters,
            ..Default::default()
        };

        let mut stream = docker.events(Some(options));

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    return Ok(());
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(event)) => {
                            self.handle_docker_event(docker, &event, sender).await;
                        }
                        Some(Err(e)) => {
                            return Err(CoreError::DockerConnection(e.to_string()));
                        }
                        None => {
                            return Err(CoreError::DockerConnection("event stream ended".to_string()));
                        }
                    }
                }
            }
        }
    }

    /// Handles a single Docker event.
    async fn handle_docker_event(
        &self,
        docker: &Docker,
        event: &bollard::models::EventMessage,
        sender: &mpsc::Sender<ProviderEvent>,
    ) {
        let action = match &event.action {
            Some(a) => a.as_str(),
            None => return,
        };

        let container_id = match &event.actor {
            Some(actor) => match &actor.id {
                Some(id) => id.as_str(),
                None => return,
            },
            None => return,
        };

        let container_name = event
            .actor
            .as_ref()
            .and_then(|a| a.attributes.as_ref())
            .and_then(|attrs| attrs.get("name"))
            .cloned()
            .unwrap_or_else(|| container_id[..12.min(container_id.len())].to_string());

        match action {
            "start" | "unpause" => {
                match self
                    .inspect_and_build(docker, container_id, &container_name)
                    .await
                {
                    Ok(Some(pc)) => {
                        let mut known = self.known.lock().await;
                        let is_update = known.contains_key(&container_name);
                        known.insert(container_name.clone(), pc.config.clone());

                        let event = if is_update {
                            ProviderEvent::Updated(pc)
                        } else {
                            ProviderEvent::Added(pc)
                        };
                        let _ = sender.send(event).await;
                    }
                    Ok(None) => {} // Not an infrarust container
                    Err(e) => {
                        tracing::warn!(
                            container = %container_name,
                            error = %e,
                            "failed to inspect started container"
                        );
                    }
                }
            }
            "stop" | "die" | "pause" | "destroy" => {
                let mut known = self.known.lock().await;
                if known.remove(&container_name).is_some() {
                    let _ = sender
                        .send(ProviderEvent::Removed(ProviderId::docker(&container_name)))
                        .await;
                }
            }
            _ => {}
        }
    }

    /// Re-scans all containers and emits diffs against known state.
    async fn resync_containers(&self, docker: &Docker, sender: &mpsc::Sender<ProviderEvent>) {
        let current = match self.scan_containers(docker).await {
            Ok(configs) => configs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to resync containers");
                return;
            }
        };

        let mut known = self.known.lock().await;

        let current_names: HashMap<String, ProviderConfig> = current
            .into_iter()
            .map(|pc| (pc.id.unique_id.clone(), pc))
            .collect();

        // Check for new / updated
        for (name, pc) in &current_names {
            if known.contains_key(name) {
                let _ = sender.send(ProviderEvent::Updated(pc.clone())).await;
            } else {
                let _ = sender.send(ProviderEvent::Added(pc.clone())).await;
            }
            known.insert(name.clone(), pc.config.clone());
        }

        // Check for removed
        let removed: Vec<String> = known
            .keys()
            .filter(|name| !current_names.contains_key(*name))
            .cloned()
            .collect();
        for name in removed {
            known.remove(&name);
            let _ = sender
                .send(ProviderEvent::Removed(ProviderId::docker(&name)))
                .await;
        }
    }
}

impl ConfigProvider for DockerProvider {
    fn provider_type(&self) -> &str {
        "docker"
    }

    fn load_initial(
        &self,
    ) -> Pin<
        Box<dyn std::future::Future<Output = Result<Vec<ProviderConfig>, CoreError>> + Send + '_>,
    > {
        Box::pin(async move {
            let docker = self.connect()?;
            let configs = self.scan_containers(&docker).await?;

            // Store in known map
            let mut known = self.known.lock().await;
            for pc in &configs {
                known.insert(pc.id.unique_id.clone(), pc.config.clone());
            }

            tracing::info!(
                count = configs.len(),
                "docker provider loaded initial configs"
            );
            Ok(configs)
        })
    }

    fn watch(
        &self,
        sender: mpsc::Sender<ProviderEvent>,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async move { self.watch_with_reconnect(&sender, &shutdown).await })
    }
}

// ---------------------------------------------------------------------------
// Label parsing and address resolution
// ---------------------------------------------------------------------------

/// Converts Docker container labels to a `ServerConfig`.
pub fn labels_to_server_config(
    container_name: &str,
    labels: &HashMap<String, String>,
    address: &str,
) -> ServerConfig {
    let domains = labels
        .get("infrarust.domains")
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_else(|| vec![format!("{container_name}.docker.local")]);

    let proxy_mode = labels
        .get("infrarust.proxy_mode")
        .and_then(|m| match m.as_str() {
            "passthrough" => Some(ProxyMode::Passthrough),
            "client_only" => Some(ProxyMode::ClientOnly),
            "offline" => Some(ProxyMode::Offline),
            "server_only" => Some(ProxyMode::ServerOnly),
            "zero_copy" => Some(ProxyMode::ZeroCopy),
            _ => None,
        })
        .unwrap_or(ProxyMode::Passthrough);

    let send_proxy_protocol = labels
        .get("infrarust.send_proxy_protocol")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let server_address: ServerAddress = address.parse().unwrap_or_else(|_| ServerAddress {
        host: address.to_string(),
        port: DEFAULT_MC_PORT,
    });

    // Build a TOML string and parse it to satisfy deny_unknown_fields
    // This ensures we get proper defaults for all fields.
    let mut toml_str = format!(
        "domains = [{}]\naddresses = [\"{}\"]\nproxy_mode = \"{:?}\"\nsend_proxy_protocol = {}",
        domains
            .iter()
            .map(|d| format!("\"{}\"", d))
            .collect::<Vec<_>>()
            .join(", "),
        server_address,
        proxy_mode,
        send_proxy_protocol,
    );

    // Add MOTD text if specified
    if let Some(motd_text) = labels.get("infrarust.motd.text") {
        toml_str.push_str(&format!("\n\n[motd.online]\ntext = \"{}\"", motd_text));
    }

    // Try to parse, fall back to manual construction
    toml::from_str::<ServerConfig>(&toml_str).unwrap_or_else(|_| {
        // Manual fallback — less complete but functional
        ServerConfig {
            id: Some(container_name.to_string()),
            domains,
            addresses: vec![server_address],
            proxy_mode,
            send_proxy_protocol,
            domain_rewrite: Default::default(),
            motd: Default::default(),
            server_manager: None,
            timeouts: None,
            max_players: 0,
            ip_filter: None,
            disconnect_message: None,
        }
    })
}

/// Resolves the best address for a Docker container.
///
/// Priority:
/// 1. Network IP from the preferred network (or first available)
/// 2. Port bindings → `host_ip:host_port`
/// 3. Container name as hostname
pub fn resolve_container_address(
    info: &ContainerInspectResponse,
    preferred_network: Option<&str>,
    port: u16,
) -> String {
    // 1. Network IP
    if let Some(networks) = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.networks.as_ref())
    {
        // Try preferred network first
        if let Some(net_name) = preferred_network
            && let Some(net) = networks.get(net_name)
            && let Some(ip) = &net.ip_address
            && !ip.is_empty()
        {
            return format!("{ip}:{port}");
        }
        // Try any network
        for net in networks.values() {
            if let Some(ip) = &net.ip_address
                && !ip.is_empty()
            {
                return format!("{ip}:{port}");
            }
        }
    }

    // 2. Port bindings
    if let Some(bindings) = info
        .host_config
        .as_ref()
        .and_then(|hc| hc.port_bindings.as_ref())
    {
        let key = format!("{port}/tcp");
        if let Some(Some(binding_list)) = bindings.get(&key)
            && let Some(binding) = binding_list.first()
        {
            let host_port = binding.host_port.as_deref().unwrap_or("25565");
            let host_ip = binding.host_ip.as_deref().unwrap_or("0.0.0.0");
            let actual_ip = if host_ip == "0.0.0.0" {
                "127.0.0.1"
            } else {
                host_ip
            };
            return format!("{actual_ip}:{host_port}");
        }
    }

    // 3. Container name
    let name = info
        .name
        .as_deref()
        .unwrap_or("unknown")
        .trim_start_matches('/');
    format!("{name}:{port}")
}
