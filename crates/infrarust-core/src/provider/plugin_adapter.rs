use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{
    PluginConfigProvider, PluginProviderEvent, PluginProviderSender, ServerDocument,
};

use crate::provider::provider_id::ProviderId;
use crate::provider::traits::{ProviderConfig, ProviderEvent};
use crate::routing::DomainRouter;
use crate::util::sync::lock;

struct PluginProviderSenderImpl {
    sender: mpsc::Sender<ProviderEvent>,
    shutdown: CancellationToken,
    plugin_id: String,
    kind: String,
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
                PluginProviderEvent::Removed(server_id) => {
                    Some(ProviderEvent::Removed(self.provider_id(server_id.as_str())))
                }
                _ => {
                    tracing::warn!(
                        plugin = %self.plugin_id,
                        provider = %self.kind,
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
    fn provider_id(&self, document: &str) -> ProviderId {
        ProviderId::plugin(&self.plugin_id, &self.kind, document)
    }

    fn provider_config_from(&self, doc: &ServerDocument) -> Option<ProviderConfig> {
        match parse_document(doc) {
            Ok(config) => Some(ProviderConfig {
                id: self.provider_id(doc.id.as_str()),
                config,
            }),
            Err(e) => {
                tracing::warn!(
                    plugin = %self.plugin_id,
                    provider = %self.kind,
                    document = %doc.id.as_str(),
                    error = %e,
                    "rejecting invalid server document"
                );
                None
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DocumentError {
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    Config(#[from] infrarust_config::ConfigError),
}

pub fn parse_document(
    doc: &ServerDocument,
) -> Result<infrarust_config::ServerConfig, DocumentError> {
    let mut config: infrarust_config::ServerConfig = toml::from_str(&doc.toml)?;

    if config.id.is_none() {
        config.id = Some(doc.id.as_str().to_string());
    }

    for warning in infrarust_config::validate_server_config(&config)? {
        tracing::warn!(server = %config.effective_id(), "{warning}");
    }
    Ok(config)
}

struct Activation {
    config_ids: Vec<ProviderId>,
    watch: CancellationToken,
}

pub struct PluginProviderActivator {
    sender: mpsc::Sender<ProviderEvent>,
    router: Arc<DomainRouter>,
    shutdown: CancellationToken,
    started: AtomicBool,
    active: Mutex<HashMap<String, Vec<Activation>>>,
}

impl PluginProviderActivator {
    pub fn new(
        sender: mpsc::Sender<ProviderEvent>,
        router: Arc<DomainRouter>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            sender,
            router,
            shutdown,
            started: AtomicBool::new(false),
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn start(&self) {
        self.started.store(true, Ordering::SeqCst);
    }

    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    pub fn spawn(self: &Arc<Self>, plugin_id: &str, provider: Box<dyn PluginConfigProvider>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                plugin = %plugin_id,
                provider = provider.provider_type(),
                "config provider registered outside the async runtime: dropped"
            );
            return;
        };
        let activator = Arc::clone(self);
        let plugin_id = plugin_id.to_owned();
        runtime.spawn(async move { activator.activate(&plugin_id, provider).await });
    }

    pub async fn activate(&self, plugin_id: &str, provider: Box<dyn PluginConfigProvider>) {
        let kind = provider.provider_type().to_string();
        let watch = self.shutdown.child_token();
        self.record(
            plugin_id,
            Activation {
                config_ids: Vec::new(),
                watch: watch.clone(),
            },
        );

        let documents = match provider.load_initial().await {
            Ok(documents) => documents,
            Err(e) => {
                tracing::warn!(
                    plugin = %plugin_id,
                    provider = %kind,
                    error = %e,
                    "plugin config provider failed to load initial documents"
                );
                Vec::new()
            }
        };

        {
            let mut active = lock(&self.active);
            if watch.is_cancelled() {
                return;
            }
            let mut config_ids = Vec::new();
            for doc in &documents {
                match parse_document(doc) {
                    Ok(server_config) => {
                        let pid = ProviderId::plugin(plugin_id, &kind, doc.id.as_str());
                        self.router.add(pid.clone(), server_config);
                        config_ids.push(pid);
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
                provider = %kind,
                count = config_ids.len(),
                "plugin config provider loaded initial documents"
            );
            active
                .entry(plugin_id.to_owned())
                .or_default()
                .push(Activation {
                    config_ids,
                    watch: watch.clone(),
                });
        }

        let sender_impl = Box::new(PluginProviderSenderImpl {
            sender: self.sender.clone(),
            shutdown: watch,
            plugin_id: plugin_id.to_owned(),
            kind: kind.clone(),
        });
        let plugin_id = plugin_id.to_owned();
        tokio::spawn(async move {
            if let Err(e) = provider.watch(sender_impl).await {
                tracing::warn!(
                    plugin = %plugin_id,
                    provider = %kind,
                    error = %e,
                    "plugin config provider watch exited with error"
                );
            }
        });
    }

    pub fn deactivate(&self, plugin_id: &str) {
        let Some(activations) = lock(&self.active).remove(plugin_id) else {
            return;
        };
        for activation in activations {
            activation.watch.cancel();
            for pid in &activation.config_ids {
                self.router.remove(pid);
            }
        }
    }

    fn record(&self, plugin_id: &str, activation: Activation) {
        lock(&self.active)
            .entry(plugin_id.to_owned())
            .or_default()
            .push(activation);
    }
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
                plugin_id: "test".to_string(),
                kind: "api".to_string(),
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
