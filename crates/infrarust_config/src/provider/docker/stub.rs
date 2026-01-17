/// Stub DockerProvider when docker feature is not enabled
pub struct DockerProvider;

impl DockerProvider {
    pub fn new(
        _config: crate::models::infrarust::DockerProviderConfig,
        _sender: tokio::sync::mpsc::Sender<crate::provider::ProviderMessage>,
    ) -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl crate::provider::Provider for DockerProvider {
    async fn run(&mut self) {
        tracing::error!(
            log_type = "config_provider",
            "Docker provider is not enabled. Enable the 'docker' feature to use it."
        );
    }

    fn get_name(&self) -> String {
        "DockerProvider(disabled)".to_string()
    }

    fn new(_sender: tokio::sync::mpsc::Sender<crate::provider::ProviderMessage>) -> Self {
        Self
    }
}
