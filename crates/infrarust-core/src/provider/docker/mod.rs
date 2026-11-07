//! Docker-based configuration provider.
//!
//! Discovers Minecraft servers from Docker containers with `infrarust.*`
//! labels and watches Docker events for real-time updates.
//!
//! Entirely feature-gated behind `docker`.

pub mod labels;

pub use labels::{
    DomainsNotAllowed, container_server_config, labels_to_server_config, resolve_container_address,
};

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::time::Duration;

use bollard::Docker;
use bollard::query_parameters::{
    EventsOptionsBuilder, InspectContainerOptions, ListContainersOptionsBuilder,
};
use futures_util::StreamExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use infrarust_config::{DockerProviderConfig, ServerConfig};

use crate::error::CoreError;
use crate::provider::{ConfigProvider, ProviderConfig, ProviderEvent, ProviderId};

struct ContainerScan {
    configs: Vec<ProviderConfig>,
    unreadable: HashSet<String>,
}

/// Default Minecraft port.
/// Docker provider that auto-discovers containers with `infrarust.*` labels.
pub struct DockerProvider {
    config: DockerProviderConfig,
    /// Known containers: container_name → ServerConfig
    known: Mutex<HashMap<String, ServerConfig>>,
    /// Last refusal logged per container, so polling does not repeat it.
    refusals: Mutex<HashMap<String, String>>,
}

impl DockerProvider {
    pub fn new(config: &DockerProviderConfig) -> Self {
        Self {
            config: config.clone(),
            known: Mutex::new(HashMap::new()),
            refusals: Mutex::new(HashMap::new()),
        }
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
    async fn scan_containers(&self, docker: &Docker) -> Result<ContainerScan, CoreError> {
        let mut filters = HashMap::new();
        filters.insert("label", vec!["infrarust.enable=true"]);
        filters.insert("status", vec!["running"]);

        let options = ListContainersOptionsBuilder::default()
            .all(false)
            .filters(&filters)
            .build();

        let containers = docker
            .list_containers(Some(options))
            .await
            .map_err(|e| CoreError::DockerConnection(e.to_string()))?;

        let mut configs = Vec::new();
        let mut unreadable = HashSet::new();
        let mut listed = HashSet::new();

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
                .unwrap_or_else(|| container_id[..12.min(container_id.len())].to_string());
            listed.insert(container_name.clone());

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
                    unreadable.insert(container_name);
                }
            }
        }

        self.refusals
            .lock()
            .await
            .retain(|name, _| listed.contains(name));

        Ok(ContainerScan {
            configs,
            unreadable,
        })
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
            .unwrap_or(infrarust_config::DEFAULT_MC_PORT);

        let Some(address) = resolve_container_address(&info, self.config.network.as_deref(), port)
        else {
            self.log_refusal(
                container_name,
                format!(
                    "container not routed: it has no network IP and does not publish port {port}"
                ),
            )
            .await;
            return Ok(None);
        };

        let config = match container_server_config(&self.config, container_name, labels, &address) {
            Ok(config) => config,
            Err(DomainsNotAllowed { allowing_line }) => {
                self.log_refusal(
                    container_name,
                    format!(
                        "container not routed: its infrarust.domains label claims domains \
                         outside the allowlist. To allow them, set this line in [docker]: \
                         {allowing_line}"
                    ),
                )
                .await;
                return Ok(None);
            }
        };
        self.refusals.lock().await.remove(container_name);

        let warnings = match infrarust_config::validate_server_config(&config) {
            Ok(warnings) => warnings,
            Err(e) => {
                tracing::warn!(
                    container = %container_name,
                    error = %e,
                    "skipping container with invalid config"
                );
                return Ok(None);
            }
        };
        for warning in warnings {
            tracing::warn!(container = %container_name, "{warning}");
        }

        Ok(Some(ProviderConfig {
            id: ProviderId::docker(container_name),
            config,
        }))
    }

    async fn log_refusal(&self, container_name: &str, reason: String) {
        let mut refusals = self.refusals.lock().await;
        if refusals.get(container_name) != Some(&reason) {
            tracing::warn!(container = %container_name, "{reason}");
            refusals.insert(container_name.to_string(), reason);
        }
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

            reconnect_delay = self.config.reconnect_delay;

            match self.watch_events(&docker, sender, shutdown).await {
                Ok(()) => return Ok(()),
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
                }
            }
        }
    }

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

        let options = EventsOptionsBuilder::default().filters(&filters).build();

        let mut stream = docker.events(Some(options));

        self.resync_containers(docker, sender).await;

        let mut poll = tokio::time::interval(self.config.poll_interval);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        poll.tick().await;

        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    return Ok(());
                }
                _ = poll.tick() => {
                    self.resync_containers(docker, sender).await;
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
        let scan = match self.scan_containers(docker).await {
            Ok(scan) => scan,
            Err(e) => {
                tracing::warn!(error = %e, "failed to resync containers");
                return;
            }
        };

        let mut known = self.known.lock().await;
        for event in reconcile(&mut known, scan) {
            let _ = sender.send(event).await;
        }
    }
}

fn reconcile(known: &mut HashMap<String, ServerConfig>, scan: ContainerScan) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    let mut seen = scan.unreadable;

    for pc in scan.configs {
        let name = pc.id.unique_id.clone();
        seen.insert(name.clone());
        match known.get(&name) {
            Some(config) if *config == pc.config => {}
            previous => {
                let is_update = previous.is_some();
                known.insert(name, pc.config.clone());
                events.push(if is_update {
                    ProviderEvent::Updated(pc)
                } else {
                    ProviderEvent::Added(pc)
                });
            }
        }
    }

    known.retain(|name, _| {
        let keep = seen.contains(name);
        if !keep {
            events.push(ProviderEvent::Removed(ProviderId::docker(name)));
        }
        keep
    });

    events
}

impl ConfigProvider for DockerProvider {
    fn provider_type(&self) -> &str {
        "docker"
    }

    fn load_initial(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ProviderConfig>, CoreError>> + Send + '_>> {
        Box::pin(async move {
            let docker = self.connect()?;
            let ContainerScan { configs, .. } = self.scan_containers(&docker).await?;

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
    ) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async move { self.watch_with_reconnect(&sender, &shutdown).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(name: &str, address: &str) -> ServerConfig {
        labels_to_server_config(name, &HashMap::new(), address)
    }

    fn provider_config(name: &str, address: &str) -> ProviderConfig {
        ProviderConfig {
            id: ProviderId::docker(name),
            config: config(name, address),
        }
    }

    fn scan(configs: Vec<ProviderConfig>, unreadable: &[&str]) -> ContainerScan {
        ContainerScan {
            configs,
            unreadable: unreadable.iter().map(ToString::to_string).collect(),
        }
    }

    fn known(entries: &[(&str, &str)]) -> HashMap<String, ServerConfig> {
        entries
            .iter()
            .map(|(name, address)| (name.to_string(), config(name, address)))
            .collect()
    }

    #[test]
    fn reconcile_is_silent_when_nothing_changed() {
        let mut known = known(&[("mc", "10.0.0.1:25565")]);
        let events = reconcile(
            &mut known,
            scan(vec![provider_config("mc", "10.0.0.1:25565")], &[]),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn reconcile_adds_unknown_containers() {
        let mut known = HashMap::new();
        let events = reconcile(
            &mut known,
            scan(vec![provider_config("mc", "10.0.0.1:25565")], &[]),
        );
        assert!(matches!(events.as_slice(), [ProviderEvent::Added(pc)] if pc.id.unique_id == "mc"));
        assert!(known.contains_key("mc"));
    }

    #[test]
    fn reconcile_updates_changed_containers() {
        let mut known = known(&[("mc", "10.0.0.1:25565")]);
        let new = provider_config("mc", "10.0.0.2:25565");
        let events = reconcile(&mut known, scan(vec![new.clone()], &[]));
        assert!(
            matches!(events.as_slice(), [ProviderEvent::Updated(pc)] if pc.id.unique_id == "mc")
        );
        assert_eq!(known.get("mc"), Some(&new.config));
    }

    #[test]
    fn reconcile_removes_containers_no_longer_listed() {
        let mut known = known(&[("mc", "10.0.0.1:25565")]);
        let events = reconcile(&mut known, scan(Vec::new(), &[]));
        assert!(matches!(events.as_slice(), [ProviderEvent::Removed(id)] if id.unique_id == "mc"));
        assert!(known.is_empty());
    }

    #[test]
    fn reconcile_keeps_containers_that_could_not_be_inspected() {
        let mut known = known(&[("mc", "10.0.0.1:25565")]);
        let events = reconcile(&mut known, scan(Vec::new(), &["mc"]));
        assert!(events.is_empty());
        assert!(known.contains_key("mc"));
    }
}
