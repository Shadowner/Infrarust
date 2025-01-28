use log::{debug, error};
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
    pub fn new(
        config_service: Arc<ConfigurationService>,
        provider_receiver: Receiver<ProviderMessage>,
        provider_sender: Sender<ProviderMessage>,
    ) -> Self {
        Self {
            _providers: vec![],
            config_service,
            provider_receiver,
            provider_sender,
        }
    }

    pub async fn run(&mut self) {
        debug!("Starting ConfigProvider(run)");
        while let Some(message) = self.provider_receiver.recv().await {
            debug!("Received message in ConfigProvider(run): {:?}", message);
            match message {
                ProviderMessage::Update { key, configuration } => {
                    debug!("Configuration update received for key: {}", key);
                    match configuration {
                        Some(config) => {
                            self.config_service
                                .update_configurations(vec![config])
                                .await;
                        }
                        None => {
                            self.config_service.remove_configuration(&key).await;
                        }
                    }
                }
                ProviderMessage::FirstInit(configs) => {
                    debug!("First init received for configs: {:?}", configs.keys());
                    let config_vec = configs.into_values().collect();
                    self.config_service.update_configurations(config_vec).await;
                }
                ProviderMessage::Error(err) => {
                    error!("Provider error: {}", err);
                }
                ProviderMessage::Shutdown => break,
            }
        }
    }

    pub fn register_provider(&mut self, mut provider: Box<dyn Provider>) {
        debug!("Registering provider: {}", provider.get_name());
        let sender = self.provider_sender.clone();
        tokio::spawn(async move {
            match provider.run().await {
                _ => {
                    debug!("Provider finished: {}", provider.get_name());
                    sender
                        .send(ProviderMessage::Error(
                            "Unexpected end for provider".to_string(),
                        ))
                        .await
                        .unwrap();
                }
            };
        });
    }
}
