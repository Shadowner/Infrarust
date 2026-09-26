use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{
    PluginConfigProvider, PluginProviderEvent, PluginProviderSender, ServerDocument,
};

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
                PluginProviderEvent::Added(doc) => {
                    self.provider_config_from(&doc).map(ProviderEvent::Added)
                }
                PluginProviderEvent::Updated(doc) => {
                    self.provider_config_from(&doc).map(ProviderEvent::Updated)
                }
                PluginProviderEvent::Removed(server_id) => Some(ProviderEvent::Removed(
                    make_provider_id(&self.provider_prefix, server_id.as_str()),
                )),
                _ => {
                    tracing::warn!(
                        provider = %self.provider_prefix,
                        "dropping unknown PluginProviderEvent variant"
                    );
                    None
                }
            };

            // A rejected document must not look like a closed channel, or the
            // plugin would stop watching.
            let Some(core_event) = core_event else {
                return !self.sender.is_closed();
            };
            self.sender.send(core_event).await.is_ok()
        })
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.is_cancelled()
    }
}

impl PluginProviderSenderImpl {
    fn provider_config_from(&self, doc: &ServerDocument) -> Option<ProviderConfig> {
        match parse_document(doc) {
            Ok(config) => Some(ProviderConfig {
                id: make_provider_id(&self.provider_prefix, doc.id.as_str()),
                config,
            }),
            Err(e) => {
                tracing::warn!(
                    provider = %self.provider_prefix,
                    document = %doc.id.as_str(),
                    error = %e,
                    "rejecting invalid server document"
                );
                None
            }
        }
    }
}

/// Parses a plugin-supplied TOML document into a full server config.
///
/// # Errors
///
/// Returns a human-readable message when the TOML does not parse or the
/// resulting config fails [`infrarust_config::validate_server_config`].
pub fn parse_document(doc: &ServerDocument) -> Result<infrarust_config::ServerConfig, String> {
    let mut config: infrarust_config::ServerConfig =
        toml::from_str(&doc.toml).map_err(|e| e.to_string())?;

    if config.id.is_none() {
        config.id = Some(doc.id.as_str().to_string());
    }

    log_validation_warnings(&config)?;
    Ok(config)
}

fn log_validation_warnings(config: &infrarust_config::ServerConfig) -> Result<(), String> {
    let warnings = infrarust_config::validate_server_config(config).map_err(|e| e.to_string())?;
    for warning in warnings {
        tracing::warn!(server = %config.effective_id(), "{warning}");
    }
    Ok(())
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
            Ok(documents) => {
                let mut count = 0;
                for doc in &documents {
                    match parse_document(doc) {
                        Ok(server_config) => {
                            let pid = make_provider_id(&provider_prefix, doc.id.as_str());
                            domain_router.add(pid.clone(), server_config);
                            loaded_ids.push(pid);
                            count += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                plugin = %plugin_id,
                                document = %doc.id.as_str(),
                                error = %e,
                                "skipping invalid server document"
                            );
                        }
                    }
                }
                tracing::info!(
                    plugin = %plugin_id,
                    provider = provider.provider_type(),
                    count,
                    "plugin config provider loaded initial documents"
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin_id,
                    provider = provider.provider_type(),
                    error = %e,
                    "plugin config provider failed to load initial documents"
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use infrarust_api::types::ServerId;

    use super::*;

    fn document(id: &str, toml: &str) -> ServerDocument {
        ServerDocument {
            id: ServerId::new(id),
            toml: toml.to_string(),
        }
    }

    #[test]
    fn parse_document_preserves_every_field() {
        let doc = document(
            "survival",
            r#"
                domains = ["mc.example.com"]
                addresses = ["10.0.0.1:25565", { address = "10.0.0.2:25565", weight = 3 }]
                balance = "least_conn"
                slow_start = "45s"
                slow_start_aggression = 2.5
                proxy_mode = "client_only"
                max_players = 100

                [active_health]
                kind = "status_ping"
                probe_healthy = true
                interval = "7s"

                [motd.online]
                text = "Welcome"
                max_players = 42
            "#,
        );

        let config = parse_document(&doc).unwrap();

        assert_eq!(config.balance, infrarust_config::BalanceStrategy::LeastConn);
        assert_eq!(config.slow_start, Some(std::time::Duration::from_secs(45)));
        assert!((config.slow_start_aggression - 2.5).abs() < f64::EPSILON);
        assert_eq!(config.addresses[1].weight, 3);
        assert_eq!(config.addresses[1].address.port, 25565);
        assert_eq!(config.max_players, 100);

        let health = config.active_health.as_ref().unwrap();
        assert_eq!(health.kind, infrarust_config::ProbeKind::StatusPing);
        assert!(health.probe_healthy);
        assert_eq!(health.interval, std::time::Duration::from_secs(7));

        let motd = config.motd.online.as_ref().unwrap();
        assert_eq!(motd.text, "Welcome");
        assert_eq!(motd.max_players, Some(42));

        let reparsed = parse_document(&document(
            "survival",
            &toml::to_string_pretty(&config).unwrap(),
        ))
        .unwrap();
        assert_eq!(config, reparsed);
    }

    #[test]
    fn parse_document_inherits_id_from_document() {
        let doc = document(
            "lobby",
            r#"
                domains = ["lobby.example.com"]
                addresses = ["10.0.0.1:25565"]
            "#,
        );
        assert_eq!(parse_document(&doc).unwrap().effective_id(), "lobby");
    }

    #[test]
    fn parse_document_keeps_explicit_id() {
        let doc = document(
            "file-stem",
            r#"
                id = "explicit"
                domains = ["explicit.example.com"]
                addresses = ["10.0.0.1:25565"]
            "#,
        );
        assert_eq!(parse_document(&doc).unwrap().effective_id(), "explicit");
    }

    fn test_sender() -> (PluginProviderSenderImpl, mpsc::Receiver<ProviderEvent>) {
        let (sender, receiver) = mpsc::channel(4);
        (
            PluginProviderSenderImpl {
                sender,
                shutdown: CancellationToken::new(),
                provider_prefix: "plugin:test:api".to_string(),
            },
            receiver,
        )
    }

    #[tokio::test]
    async fn rejected_documents_are_dropped_without_closing_the_sender() {
        let (sender, mut receiver) = test_sender();

        assert!(
            sender
                .send(PluginProviderEvent::Added(document(
                    "no-addresses",
                    "addresses = []"
                )))
                .await
        );
        assert!(receiver.try_recv().is_err());

        assert!(
            sender
                .send(PluginProviderEvent::Added(document(
                    "lobby",
                    "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]"
                )))
                .await
        );
        assert!(matches!(
            receiver.try_recv(),
            Ok(ProviderEvent::Added(config)) if config.config.effective_id() == "lobby"
        ));

        assert!(
            sender
                .send(PluginProviderEvent::Updated(document(
                    "Bad Id",
                    "domains = [\"bad.example.com\"]\naddresses = [\"10.0.0.1:25565\"]"
                )))
                .await
        );
        assert!(receiver.try_recv().is_err());

        drop(receiver);
        assert!(
            !sender
                .send(PluginProviderEvent::Removed(ServerId::new("lobby")))
                .await
        );
    }

    #[test]
    fn parse_document_rejects_malformed_toml() {
        assert!(parse_document(&document("broken", "addresses = [")).is_err());
    }

    #[test]
    fn parse_document_rejects_unknown_field() {
        let doc = document("broken", "addresses = [\"1.2.3.4:1\"]\nnot_a_field = 1");
        assert!(parse_document(&doc).is_err());
    }

    #[test]
    fn parse_document_rejects_invalid_config() {
        assert!(parse_document(&document("empty", "addresses = []")).is_err());
    }
}
