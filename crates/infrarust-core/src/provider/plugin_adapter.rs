use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{PluginConfigProvider, PluginProviderEvent, PluginProviderSender};

use crate::provider::provider_id::ProviderId;
use crate::provider::traits::{ProviderConfig, ProviderEvent};
use crate::routing::DomainRouter;

pub struct ActivatedProvider {
    pub config_ids: Vec<ProviderId>,
    pub watch_token: CancellationToken,
}

struct PluginProviderSenderImpl {
    sender: mpsc::Sender<ProviderEvent>,
    shutdown: CancellationToken,
    provider_prefix: String,
}

impl PluginProviderSender for PluginProviderSenderImpl {
    fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let core_event = match event {
                PluginProviderEvent::Added(config) => {
                    let id = config.id.as_str().to_string();
                    ProviderEvent::Added(ProviderConfig {
                        id: make_provider_id(&self.provider_prefix, &id),
                        config: convert_api_to_config(&config),
                    })
                }
                PluginProviderEvent::Updated(config) => {
                    let id = config.id.as_str().to_string();
                    ProviderEvent::Updated(ProviderConfig {
                        id: make_provider_id(&self.provider_prefix, &id),
                        config: convert_api_to_config(&config),
                    })
                }
                PluginProviderEvent::Removed(server_id) => ProviderEvent::Removed(
                    make_provider_id(&self.provider_prefix, server_id.as_str()),
                ),
            };
            self.sender.send(core_event).await.is_ok()
        })
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.is_cancelled()
    }
}

fn convert_api_to_config(
    api: &infrarust_api::services::config_service::ServerConfig,
) -> infrarust_config::ServerConfig {
    use infrarust_api::services::config_service::ProxyMode as ApiMode;
    use infrarust_config::ProxyMode as ConfigMode;

    let proxy_mode = match api.proxy_mode {
        ApiMode::Passthrough => ConfigMode::Passthrough,
        ApiMode::ZeroCopy => ConfigMode::ZeroCopy,
        ApiMode::ClientOnly => ConfigMode::ClientOnly,
        ApiMode::Offline => ConfigMode::Offline,
        ApiMode::ServerOnly => ConfigMode::ServerOnly,
        _ => ConfigMode::Passthrough,
    };

    let addresses = api
        .addresses
        .iter()
        .map(|a| infrarust_config::ServerAddress {
            host: a.host.clone(),
            port: a.port,
        })
        .collect();

    infrarust_config::ServerConfig {
        id: Some(api.id.as_str().to_string()),
        name: None,
        network: api.network.clone(),
        domains: api.domains.clone(),
        addresses,
        proxy_mode,
        forwarding_mode: None,
        send_proxy_protocol: api.send_proxy_protocol,
        domain_rewrite: Default::default(),
        motd: Default::default(),
        server_manager: None,
        timeouts: None,
        max_players: api.max_players,
        ip_filter: None,
        disconnect_message: api.disconnect_message.clone(),
        limbo_handlers: api.limbo_handlers.clone(),
    }
}

fn make_provider_id(provider_prefix: &str, config_id: &str) -> ProviderId {
    ProviderId::new(provider_prefix, config_id)
}

pub async fn activate_plugin_providers(
    providers: Vec<(String, Box<dyn PluginConfigProvider>)>,
    event_sender: mpsc::Sender<ProviderEvent>,
    domain_router: &Arc<DomainRouter>,
    shutdown: CancellationToken,
) -> Vec<(String, ActivatedProvider)> {
    let mut results = Vec::new();

    for (plugin_id, provider) in providers {
        let provider_prefix = format!("plugin:{}:{}", plugin_id, provider.provider_type());
        let mut loaded_ids = Vec::new();

        match provider.load_initial().await {
            Ok(configs) => {
                let count = configs.len();
                for config in &configs {
                    let pid = make_provider_id(&provider_prefix, config.id.as_str());
                    let server_config = convert_api_to_config(config);
                    domain_router.add(pid.clone(), server_config);
                    loaded_ids.push(pid);
                }
                tracing::info!(
                    plugin = %plugin_id,
                    provider = provider.provider_type(),
                    count,
                    "plugin config provider loaded initial configs"
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin_id,
                    provider = provider.provider_type(),
                    error = %e,
                    "plugin config provider failed to load initial configs"
                );
            }
        }

        let watch_token = shutdown.child_token();
        let sender_impl = Box::new(PluginProviderSenderImpl {
            sender: event_sender.clone(),
            shutdown: watch_token.clone(),
            provider_prefix,
        });

        let provider_type = provider.provider_type().to_string();
        let plugin_id_clone = plugin_id.clone();

        tokio::spawn(async move {
            if let Err(e) = provider.watch(sender_impl).await {
                tracing::warn!(
                    plugin = %plugin_id_clone,
                    provider = %provider_type,
                    error = %e,
                    "plugin config provider watch exited with error"
                );
            }
        });

        results.push((
            plugin_id,
            ActivatedProvider {
                config_ids: loaded_ids,
                watch_token,
            },
        ));
    }

    results
}
