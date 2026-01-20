use std::collections::HashMap;

use bollard::{
    container::ListContainersOptions,
    models::{ContainerStateStatusEnum, EventMessage},
    system::EventsOptions,
};
use futures::StreamExt;
use tracing::{debug, error, info, instrument, warn};

use super::DockerProvider;

impl DockerProvider {
    #[instrument(skip(self), name = "docker_provider: watch_events")]
    pub(crate) async fn watch_events(&self) -> Result<(), bollard::errors::Error> {
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
        info!(
            log_type = "config_provider",
            "Watching Docker events for container lifecycle changes"
        );

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
                    error!(
                        log_type = "config_provider",
                        "Error watching Docker events: {}", e
                    );
                    return Err(e);
                }
            }
        }

        warn!(log_type = "config_provider", "Docker event stream ended");
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
    pub(crate) async fn handle_docker_event(&self, event: EventMessage) {
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
                                    && let Some(container) = containers.first()
                                    && let Some(config) = self.process_container(container).await
                                {
                                    let key = self.generate_config_id(container_id);
                                    self.send_update(key, Some(config)).await;

                                    let mut tracked = self.tracked_containers.write().await;
                                    tracked.insert(container_id.to_string());
                                }
                            }
                        }
                        Err(e) => error!(
                            log_type = "config_provider",
                            "Failed to inspect container {}: {}", container_id, e
                        ),
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
}
