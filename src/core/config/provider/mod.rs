use tracing::{debug, debug_span, error, info, instrument, warn, Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::core::{config::service::ConfigurationService, event::ProviderMessage};

pub mod file;

#[async_trait::async_trait]
pub trait Provider: Send {
    async fn run(&mut self);
    fn get_name(&self) -> String;
    fn new(config_sender: Sender<ProviderMessage>) -> Self
    where
        Self: Sized;
}

pub struct ConfigProvider {
    _providers: Vec<Box<dyn Provider>>,
    config_service: Arc<ConfigurationService>,
    provider_receiver: Receiver<ProviderMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl ConfigProvider {
    #[instrument(skip(config_service, provider_receiver, provider_sender), name = "create_config_provider")]
    pub fn new(
        config_service: Arc<ConfigurationService>,
        provider_receiver: Receiver<ProviderMessage>,
        provider_sender: Sender<ProviderMessage>,
    ) -> Self {
        debug!("Creating new configuration provider");
        Self {
            _providers: vec![],
            config_service,
            provider_receiver,
            provider_sender,
        }
    }

    #[instrument(skip(self))]
    pub async fn run(&mut self) {
        let span = debug_span!("config_provider_run");
        async {
            info!("Starting configuration provider");
            while let Some(message) = self.provider_receiver.recv().await {
                self.handle_message(message)
                    .await;
            }
        }.instrument(span).await
    }

    async fn handle_message(&mut self, message: ProviderMessage) {
        match message {
            ProviderMessage::Update { key, configuration, span } => {
                // Set span parent to the span that was passed in
                let new_span = debug_span!(
                    "config_provider: config_update",
                    key = %key,
                    has_config = configuration.is_some(),
                );

                new_span.set_parent(span.context());
                
                async {
                    info!("Processing configuration update");
                    self.config_service
                        .update_configurations(vec![*configuration.unwrap()])
                        .instrument(debug_span!("config_provider: apply_config_update"))
                        .await;
                }
                .instrument(new_span)
                .await;
            }
            ProviderMessage::FirstInit(configs) => {
                debug!(config_count = configs.len(), "First initialization received");
                let config_vec = configs.into_values().collect();
                self.config_service.update_configurations(config_vec).await;
            }
            ProviderMessage::Error(err) => {
                error!(error = %err, "Provider error received");
            }
            ProviderMessage::Shutdown => {
                info!("Shutdown message received");
            }
        }
    }

    #[instrument(skip(self, provider), fields(provider_name = %provider.get_name(), name = "register_provider"))]
    pub fn register_provider(&mut self, mut provider: Box<dyn Provider>) {
        let current_span = Span::current();
        let sender = self.provider_sender.clone();
        tokio::spawn(
            async move {
                provider.run()
                    .instrument(debug_span!("provider_run"))
                    .await;
                warn!("Provider stopped unexpectedly");
                if let Err(e) = sender
                    .send(ProviderMessage::Error(
                        "Unexpected provider termination".into(),
                    ))
                    .await
                {
                    error!(error = %e, "Failed to send provider error");
                }
            }
            .instrument(current_span),
        );
    }
}
