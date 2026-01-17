#[cfg(feature = "docker")]
mod connection;
#[cfg(feature = "docker")]
mod container;
#[cfg(feature = "docker")]
mod events;
#[cfg(feature = "docker")]
mod updates;

#[cfg(not(feature = "docker"))]
mod stub;

#[cfg(not(feature = "docker"))]
pub use stub::DockerProvider;

#[cfg(feature = "docker")]
pub use docker_impl::DockerProvider;

#[cfg(feature = "docker")]
mod docker_impl {
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::Duration,
    };

    use async_trait::async_trait;
    use bollard::Docker;
    use tokio::sync::{RwLock, mpsc::Sender};
    use tracing::{Instrument, debug, debug_span, error, info, instrument};

    use crate::{
        models::infrarust::DockerProviderConfig,
        models::server::ServerConfig,
        provider::{Provider, ProviderMessage},
    };

    pub struct DockerProvider {
        pub(crate) config: DockerProviderConfig,
        pub(crate) docker: Option<Docker>,
        pub(crate) sender: Sender<ProviderMessage>,
        pub(crate) tracked_containers: Arc<RwLock<HashSet<String>>>,
        pub(crate) previous_configs: Arc<RwLock<HashMap<String, ServerConfig>>>,
    }

    impl DockerProvider {
        #[instrument(skip(sender), fields(docker_host = %config.docker_host), name = "docker_provider: new")]
        pub fn new(config: DockerProviderConfig, sender: Sender<ProviderMessage>) -> Self {
            debug!(
                log_type = "config_provider",
                "Initializing Docker provider with config: {:?}", config
            );
            Self {
                config,
                docker: None,
                sender,
                tracked_containers: Arc::new(RwLock::new(HashSet::new())),
                previous_configs: Arc::new(RwLock::new(HashMap::new())),
            }
        }
    }

    #[async_trait]
    impl Provider for DockerProvider {
        #[instrument(skip(self), name = "docker_provider: run")]
        async fn run(&mut self) {
            let span = debug_span!("docker_provider_run");

            async {
                if let Err(e) = self.connect().await {
                    error!(
                        log_type = "config_provider",
                        "Failed to connect to Docker daemon: {}", e
                    );
                    return;
                }

                match self.load_containers().await {
                    Ok(configs) => {
                        info!(
                            log_type = "config_provider",
                            "Loaded {} container configurations",
                            configs.len()
                        );

                        // Send initial configurations
                        let mut server_configs = HashMap::new();
                        for (key, config) in configs {
                            server_configs.insert(key, config);
                        }

                        if !server_configs.is_empty()
                            && let Err(e) = self
                                .sender
                                .send(ProviderMessage::FirstInit(server_configs))
                                .await
                        {
                            error!(
                                log_type = "config_provider",
                                "Failed to send initial configurations: {}", e
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            log_type = "config_provider",
                            "Failed to load containers: {}", e
                        );
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
                                error!(
                                    log_type = "config_provider",
                                    "Docker event watcher failed: {}", e
                                );
                            }
                        }
                        .instrument(event_span),
                    );
                }

                let mut interval = tokio::time::interval(Duration::from_secs(3600));
                loop {
                    interval.tick().await;
                    debug!(log_type = "config_provider", "Docker provider heartbeat");
                }
            }
            .instrument(span)
            .await
        }

        fn get_name(&self) -> String {
            "DockerProvider".to_string()
        }

        fn new(sender: tokio::sync::mpsc::Sender<ProviderMessage>) -> Self {
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
}
