use tokio::sync::mpsc::Sender;
use tracing::Span;

#[cfg(feature = "telemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod docker;
pub mod file;

// Event messages for the provider system
#[derive(Debug)]
pub enum ProviderMessage {
    Update {
        key: String,
        configuration: Option<Box<crate::models::server::ServerConfig>>,
        span: Span,
    },
    FirstInit(std::collections::HashMap<String, crate::models::server::ServerConfig>),
    Error(String),
    Shutdown,
}

#[async_trait::async_trait]
pub trait Provider: Send {
    async fn run(&mut self);
    fn get_name(&self) -> String;
    fn new(config_sender: Sender<ProviderMessage>) -> Self
    where
        Self: Sized;
}
