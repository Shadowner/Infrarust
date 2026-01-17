use bollard::Docker;
use tracing::{debug, info, instrument};

use super::DockerProvider;

impl DockerProvider {
    #[instrument(skip(self), name = "docker_provider: connect")]
    pub(crate) async fn connect(&mut self) -> Result<(), bollard::errors::Error> {
        debug!(
            log_type = "config_provider",
            "Connecting to Docker daemon: {}", self.config.docker_host
        );

        let docker = if self.config.docker_host.starts_with("unix://") {
            Docker::connect_with_socket_defaults()?
        } else if self.config.docker_host.starts_with("tcp://") {
            Docker::connect_with_http_defaults()?
        } else {
            Docker::connect_with_local_defaults()?
        };

        docker.ping().await?;
        info!(
            log_type = "config_provider",
            "Successfully connected to Docker daemon"
        );

        self.docker = Some(docker);
        Ok(())
    }
}
