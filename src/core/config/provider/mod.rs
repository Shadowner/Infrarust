use log::{debug, error};
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::core::event::{GatewayMessage, ProviderMessage};

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
    providers: Vec<Box<dyn Provider>>,
    gateway_sender: Sender<GatewayMessage>,

    provider_receiver: Receiver<ProviderMessage>,
    provider_sender: Sender<ProviderMessage>,
}

impl ConfigProvider {
    pub fn new(
        gateway_sender: Sender<GatewayMessage>,

        provider_receiver: Receiver<ProviderMessage>,
        provider_sender: Sender<ProviderMessage>,
    ) -> Self {
        Self {
            providers: vec![],
            gateway_sender,
            provider_receiver,
            provider_sender,
        }
    }

    pub fn new_and_generate_channels(gateway_sender: Sender<GatewayMessage>) -> Self {
        let (provider_sender, provider_receiver) = mpsc::channel(100);
        Self {
            providers: vec![],
            gateway_sender,
            provider_receiver,
            provider_sender,
        }
    }

    pub async fn run(&mut self) {
        debug!("Starting ConfigProvider(run)");
        while let Some(message) = self.provider_receiver.recv().await {
            match message {
                ProviderMessage::Update { key, configuration } => {
                    debug!("Configuration update received for key: {}", key);
                    self.gateway_sender
                        .send(GatewayMessage::ConfigurationUpdate {
                            key: key.clone(),
                            configuration,
                        })
                        .await
                        .unwrap();
                }
                ProviderMessage::FirstInit(configs) => {
                    debug!("First init received for configs: {:?}", configs.keys());
                    for (key, config) in configs {
                        let _ = self.gateway_sender
                            .send(GatewayMessage::ConfigurationUpdate {
                                key: key.clone(),
                                configuration: Some(config),
                            })
                            .await;
                    }
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
