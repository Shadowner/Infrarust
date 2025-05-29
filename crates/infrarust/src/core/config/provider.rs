use infrarust_config::{LogType, provider::{Provider, ProviderMessage}};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{Instrument, Span, debug, debug_span, error, info, instrument, warn};

use super::service::ConfigurationService;

pub struct ConfigProvider {
    _providers: Vec<Box<dyn Provider>>,
    config_service: Arc<ConfigurationService>,
    provider_receiver: Receiver<ProviderMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl ConfigProvider {
    #[instrument(
        skip(config_service, provider_receiver, provider_sender),
        name = "create_config_provider"
    )]
    pub fn new(
        config_service: Arc<ConfigurationService>,
        provider_receiver: Receiver<ProviderMessage>,
        provider_sender: Sender<ProviderMessage>,
    ) -> Self {
        debug!(log_type = LogType::ConfigProvider.as_str(), "Creating new configuration provider");
        Self {
            _providers: vec![],
            config_service,
            provider_receiver,
            provider_sender,
        }
    }

    #[instrument(skip(self))]
    pub async fn run(&mut self) {
        let span = debug_span!("config_provider_run", log_type = LogType::ConfigProvider.as_str());
        async {
            info!(log_type = LogType::ConfigProvider.as_str(), "Starting configuration provider");
            while let Some(message) = self.provider_receiver.recv().await {
                self.handle_message(message).await;
            }
        }
        .instrument(span)
        .await
    }

    async fn handle_message(&mut self, message: ProviderMessage) {
        match message {
            ProviderMessage::Update {
                key,
                configuration,
                span: _span,
            } => {
                // Set span parent to the span that was passed in
                let new_span = debug_span!(
                    "config_provider: config_update",
                    key = %key,
                    has_config = configuration.is_some(),
                    log_type = LogType::ConfigProvider.as_str()
                );

                #[cfg(feature = "telemetry")]
                new_span.set_parent(_span.context());

                async {
                    debug!(log_type = LogType::ConfigProvider.as_str(), "Processing configuration update for: {}", key);

                    if let Some(config) = configuration {
                        self.config_service
                            .update_configurations(vec![*config])
                            .instrument(debug_span!("config_provider: apply_config_update", log_type = LogType::ConfigProvider.as_str()))
                            .await;
                    } else {
                        self.config_service.remove_configuration(&key).await;
                    }
                }
                .instrument(new_span)
                .await;
            }
            ProviderMessage::FirstInit(configs) => {
                debug!(
                    log_type = LogType::ConfigProvider.as_str(),
                    config_count = configs.len(),
                    "First initialization received"
                );
                let config_vec = configs.into_values().collect();
                self.config_service.update_configurations(config_vec).await;
            }
            ProviderMessage::Error(err) => {
                error!(log_type = LogType::ConfigProvider.as_str(), error = %err, "Provider error received");
            }
            ProviderMessage::Shutdown => {
                info!(log_type = LogType::ConfigProvider.as_str(), "Shutdown message received");
            }
        }
    }

    #[instrument(skip(self, provider), fields(provider_name = %provider.get_name(), name = "register_provider"))]
    pub fn register_provider(&mut self, mut provider: Box<dyn Provider>) {
        let current_span = Span::current();
        let sender = self.provider_sender.clone();
        tokio::spawn(
            async move {
                provider.run().instrument(debug_span!("provider_run", log_type = LogType::ConfigProvider.as_str())).await;
                warn!(log_type = LogType::ConfigProvider.as_str(), "Provider stopped unexpectedly");
                if let Err(e) = sender
                    .send(ProviderMessage::Error(
                        "Unexpected provider termination".into(),
                    ))
                    .await
                {
                    error!(log_type = LogType::ConfigProvider.as_str(), error = %e, "Failed to send provider error");
                }
            }
            .instrument(current_span),
        );
    }
}
