use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::provider::provider_id::ProviderId;
use crate::provider::registry::PluginRoutes;
use crate::provider::traits::{ProviderChange, ProviderConfig};
use crate::util::sync::lock;
use infrarust_api::event::BoxFuture;
use infrarust_api::provider::{
    PluginConfigProvider, PluginProviderEvent, PluginProviderSender, ServerDocument,
};

struct PluginProviderSenderImpl {
    routes: PluginRoutes,
    watch: CancellationToken,
    plugin_id: String,
    kind: String,
}

impl PluginProviderSender for PluginProviderSenderImpl {
    fn send(&self, event: PluginProviderEvent) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let change = match event {
                PluginProviderEvent::Added(doc) => {
                    self.provider_config_from(&doc).map(ProviderChange::Added)
                }
                PluginProviderEvent::Updated(doc) => {
                    self.provider_config_from(&doc).map(ProviderChange::Updated)
                }
                PluginProviderEvent::Removed(server_id) => Some(ProviderChange::Removed(
                    self.provider_id(server_id.as_str()),
                )),
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
            let Some(change) = change else {
                return !self.routes.is_closed();
            };
            self.routes.submit(&self.watch, vec![change]).await
        })
    }

    fn is_shutdown(&self) -> bool {
        self.watch.is_cancelled()
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

pub struct PluginProviderActivator {
    routes: PluginRoutes,
    shutdown: CancellationToken,
    started: AtomicBool,
    watches: Mutex<HashMap<String, Vec<CancellationToken>>>,
}

impl PluginProviderActivator {
    pub fn new(routes: PluginRoutes, shutdown: CancellationToken) -> Self {
        Self {
            routes,
            shutdown,
            started: AtomicBool::new(false),
            watches: Mutex::new(HashMap::new()),
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
        lock(&self.watches)
            .entry(plugin_id.to_owned())
            .or_default()
            .push(watch.clone());

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

        let sender = PluginProviderSenderImpl {
            routes: self.routes.clone(),
            watch,
            plugin_id: plugin_id.to_owned(),
            kind,
        };
        let initial: Vec<ProviderChange> = documents
            .iter()
            .filter_map(|doc| sender.provider_config_from(doc))
            .map(ProviderChange::Added)
            .collect();
        tracing::info!(
            plugin = %sender.plugin_id,
            provider = %sender.kind,
            count = initial.len(),
            "plugin config provider loaded initial documents"
        );
        self.routes.submit_and_wait(&sender.watch, initial).await;

        tokio::spawn(async move {
            let plugin_id = sender.plugin_id.clone();
            let kind = sender.kind.clone();
            if let Err(e) = provider.watch(Box::new(sender)).await {
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
        let Some(watches) = lock(&self.watches).remove(plugin_id) else {
            return;
        };
        for watch in watches {
            watch.cancel();
        }
        self.routes.unload(plugin_id);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::time::Duration;

    use infrarust_api::types::ServerId;
    use infrarust_config::ForwardingMode;
    use tokio::sync::oneshot;

    use super::*;
    use crate::event_bus::EventBusImpl;
    use crate::provider::registry::{PluginRouteEvents, ProviderRegistry};
    use crate::routing::DomainRouter;

    struct NoListener;

    impl crate::provider::RoutesListener for NoListener {
        fn routes_changed(&self) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }

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
        assert_eq!(config.slow_start, Some(Duration::from_secs(45)));
        assert!((config.slow_start_aggression - 2.5).abs() < f64::EPSILON);
        assert_eq!(config.addresses[1].weight, 3);
        assert_eq!(config.addresses[1].address.port, 25565);
        assert_eq!(config.max_players, 100);

        let health = config.active_health.as_ref().unwrap();
        assert_eq!(health.kind, infrarust_config::ProbeKind::StatusPing);
        assert!(health.probe_healthy);
        assert_eq!(health.interval, Duration::from_secs(7));

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

    fn test_sender() -> (PluginProviderSenderImpl, PluginRouteEvents) {
        let (routes, events) = PluginRoutes::channel();
        (
            PluginProviderSenderImpl {
                routes,
                watch: CancellationToken::new(),
                plugin_id: "test".to_string(),
                kind: "api".to_string(),
            },
            events,
        )
    }

    #[tokio::test]
    async fn rejected_documents_are_dropped_without_closing_the_sender() {
        let (sender, mut events) = test_sender();

        assert!(
            sender
                .send(PluginProviderEvent::Added(document(
                    "no-addresses",
                    "addresses = []"
                )))
                .await
        );
        assert!(events.changes.try_recv().is_err());

        assert!(
            sender
                .send(PluginProviderEvent::Added(document(
                    "lobby",
                    "domains = [\"lobby.example.com\"]\naddresses = [\"10.0.0.1:25565\"]"
                )))
                .await
        );
        let submitted = events.changes.try_recv().unwrap();
        assert!(matches!(
            submitted.changes.as_slice(),
            [ProviderChange::Added(config)] if config.config.effective_id() == "lobby"
        ));

        assert!(
            sender
                .send(PluginProviderEvent::Updated(document(
                    "Bad Id",
                    "domains = [\"bad.example.com\"]\naddresses = [\"10.0.0.1:25565\"]"
                )))
                .await
        );
        assert!(events.changes.try_recv().is_err());

        drop(events);
        assert!(
            !sender
                .send(PluginProviderEvent::Removed(ServerId::new("lobby")))
                .await
        );
    }

    type WatchSender = Box<dyn PluginProviderSender>;

    struct TestProvider {
        initial: Vec<ServerDocument>,
        watching: Mutex<Option<oneshot::Sender<WatchSender>>>,
    }

    impl TestProvider {
        fn with_initial(initial: Vec<ServerDocument>) -> Box<Self> {
            Box::new(Self {
                initial,
                watching: Mutex::new(None),
            })
        }

        fn watched(initial: Vec<ServerDocument>) -> (Box<Self>, oneshot::Receiver<WatchSender>) {
            let (watching, sender) = oneshot::channel();
            let provider = Box::new(Self {
                initial,
                watching: Mutex::new(Some(watching)),
            });
            (provider, sender)
        }
    }

    impl PluginConfigProvider for TestProvider {
        fn provider_type(&self) -> &str {
            "test"
        }

        fn load_initial(
            &self,
        ) -> BoxFuture<'_, Result<Vec<ServerDocument>, infrarust_api::error::PluginError>> {
            let documents = self.initial.clone();
            Box::pin(async move { Ok(documents) })
        }

        fn watch(
            &self,
            sender: WatchSender,
        ) -> BoxFuture<'_, Result<(), infrarust_api::error::PluginError>> {
            if let Some(watching) = lock(&self.watching).take() {
                let _ = watching.send(sender);
            }
            Box::pin(async { Ok(()) })
        }
    }

    struct Proxy {
        router: Arc<DomainRouter>,
        routes: PluginRoutes,
        activator: PluginProviderActivator,
        _shutdown: tokio_util::sync::DropGuard,
    }

    impl Proxy {
        async fn start(default_forwarding: ForwardingMode) -> Self {
            let router = Arc::new(DomainRouter::new());
            let shutdown = CancellationToken::new();
            let registry = ProviderRegistry::new(
                Arc::clone(&router),
                Arc::new(EventBusImpl::new()),
                Arc::new(NoListener),
                shutdown.clone(),
                default_forwarding,
            );
            let (_, routes) = registry.start().await.unwrap();
            Self {
                router,
                activator: PluginProviderActivator::new(routes.clone(), shutdown.clone()),
                routes,
                _shutdown: shutdown.drop_guard(),
            }
        }

        async fn routed(&self) -> Vec<String> {
            self.routes
                .submit_and_wait(&CancellationToken::new(), Vec::new())
                .await;
            let mut routed: Vec<String> = self
                .router
                .list_all()
                .into_iter()
                .map(|(_, config)| config.effective_id())
                .collect();
            routed.sort();
            routed
        }
    }

    fn server(id: &str) -> ServerDocument {
        document(
            id,
            &format!("domains = [\"{id}.example.com\"]\naddresses = [\"10.0.0.1:25565\"]"),
        )
    }

    async fn activated_under(default_forwarding: ForwardingMode) -> Vec<String> {
        let proxy = Proxy::start(default_forwarding).await;
        let provider = TestProvider::with_initial(vec![
            document(
                "inherited",
                "domains = [\"inherited.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n\
                 proxy_mode = \"passthrough\"",
            ),
            document(
                "optout",
                "domains = [\"optout.example.com\"]\naddresses = [\"10.0.0.1:25565\"]\n\
                 proxy_mode = \"passthrough\"\nforwarding_mode = \"none\"",
            ),
        ]);

        proxy.activator.activate("plugin", provider).await;

        proxy.routed().await
    }

    #[tokio::test]
    async fn initial_documents_follow_the_proxy_wide_forwarding_rules() {
        for refusing in [
            ForwardingMode::BungeeCord,
            ForwardingMode::BungeeGuard,
            ForwardingMode::Velocity,
        ] {
            assert_eq!(
                activated_under(refusing.clone()).await,
                ["optout"],
                "{refusing:?}"
            );
        }
        assert_eq!(
            activated_under(ForwardingMode::None).await,
            ["inherited", "optout"]
        );
    }

    #[tokio::test]
    async fn initial_documents_are_routed_when_activation_returns() {
        let proxy = Proxy::start(ForwardingMode::None).await;

        proxy
            .activator
            .activate("plugin", TestProvider::with_initial(vec![server("lobby")]))
            .await;

        assert_eq!(proxy.router.list_all().len(), 1);
    }

    #[tokio::test]
    async fn unloading_a_plugin_removes_the_servers_its_watch_delivered() {
        let proxy = Proxy::start(ForwardingMode::None).await;
        let (provider, watching) = TestProvider::watched(vec![server("initial")]);
        proxy.activator.activate("leaving", provider).await;
        proxy
            .activator
            .activate("staying", TestProvider::with_initial(vec![server("kept")]))
            .await;
        let sender = watching.await.unwrap();

        assert!(
            sender
                .send(PluginProviderEvent::Added(server("late")))
                .await
        );
        assert_eq!(proxy.routed().await, ["initial", "kept", "late"]);

        proxy.activator.deactivate("leaving");
        assert_eq!(proxy.routed().await, ["kept"]);
    }

    #[tokio::test]
    async fn a_document_sent_after_the_unload_is_not_routed() {
        let proxy = Proxy::start(ForwardingMode::None).await;
        let (provider, watching) = TestProvider::watched(Vec::new());
        proxy.activator.activate("leaving", provider).await;
        let sender = watching.await.unwrap();

        proxy.activator.deactivate("leaving");
        assert!(sender.is_shutdown());
        sender
            .send(PluginProviderEvent::Added(server("late")))
            .await;

        assert!(proxy.routed().await.is_empty());
    }

    #[tokio::test]
    async fn a_reloaded_plugin_routes_its_new_documents() {
        let proxy = Proxy::start(ForwardingMode::None).await;
        proxy
            .activator
            .activate("plugin", TestProvider::with_initial(vec![server("old")]))
            .await;

        proxy.activator.deactivate("plugin");
        proxy
            .activator
            .activate("plugin", TestProvider::with_initial(vec![server("new")]))
            .await;

        assert_eq!(proxy.routed().await, ["new"]);
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
