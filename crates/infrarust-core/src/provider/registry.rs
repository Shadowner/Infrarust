//! `ProviderRegistry` — orchestrates config providers and feeds the `DomainRouter`.

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::events::proxy::ConfigReloadEvent;
use infrarust_api::types::ServerId;
use infrarust_config::{ForwardingMode, ServerConfig};

use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::routing::DomainRouter;

use super::{
    ConfigProvider, ProviderChange, ProviderConfig, ProviderEvent, ProviderId, RoutesListener,
};

pub(super) struct PluginChanges {
    watch: CancellationToken,
    pub(super) changes: Vec<ProviderChange>,
    applied: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub struct PluginRoutes {
    changes: mpsc::Sender<PluginChanges>,
    unloads: mpsc::UnboundedSender<String>,
}

pub struct PluginRouteEvents {
    pub(super) changes: mpsc::Receiver<PluginChanges>,
    unloads: mpsc::UnboundedReceiver<String>,
}

impl PluginRoutes {
    pub fn channel() -> (Self, PluginRouteEvents) {
        let (changes, changes_rx) = mpsc::channel(256);
        let (unloads, unloads_rx) = mpsc::unbounded_channel();
        (
            Self { changes, unloads },
            PluginRouteEvents {
                changes: changes_rx,
                unloads: unloads_rx,
            },
        )
    }

    pub async fn submit(&self, watch: &CancellationToken, changes: Vec<ProviderChange>) -> bool {
        self.changes
            .send(PluginChanges {
                watch: watch.clone(),
                changes,
                applied: None,
            })
            .await
            .is_ok()
    }

    pub async fn submit_and_wait(&self, watch: &CancellationToken, changes: Vec<ProviderChange>) {
        let (applied, done) = oneshot::channel();
        let submitted = PluginChanges {
            watch: watch.clone(),
            changes,
            applied: Some(applied),
        };
        if self.changes.send(submitted).await.is_ok() {
            let _ = done.await;
        }
    }

    pub fn unload(&self, plugin_id: &str) {
        let _ = self.unloads.send(plugin_id.to_owned());
    }

    pub fn is_closed(&self) -> bool {
        self.changes.is_closed()
    }
}

/// Orchestrates config providers, feeding their events into the `DomainRouter`.
///
/// The registry:
/// 1. Calls `load_initial()` on each provider and populates the router.
/// 2. Spawns a watch task per provider that sends `ProviderEvent`s into a
///    unified bounded channel.
/// 3. Runs an event loop that updates the router, invalidates caches, and
///    fires `ConfigReloadEvent`.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn ConfigProvider>>,
    domain_router: Arc<DomainRouter>,
    event_bus: Arc<EventBusImpl>,
    routes_listener: Arc<dyn RoutesListener>,
    shutdown: CancellationToken,
    default_forwarding: ForwardingMode,
}

impl ProviderRegistry {
    pub fn new(
        domain_router: Arc<DomainRouter>,
        event_bus: Arc<EventBusImpl>,
        routes_listener: Arc<dyn RoutesListener>,
        shutdown: CancellationToken,
        default_forwarding: ForwardingMode,
    ) -> Self {
        Self {
            providers: Vec::new(),
            domain_router,
            event_bus,
            routes_listener,
            shutdown,
            default_forwarding,
        }
    }

    /// Registers a provider. Must be called before `start()`.
    pub fn add_provider(&mut self, provider: Box<dyn ConfigProvider>) {
        self.providers.push(provider);
    }

    /// Starts all providers: loads initial configs and spawns watchers.
    ///
    /// Consumes `self` to transfer ownership of providers to spawned tasks.
    /// Returns the `JoinHandle` of the event loop task and a sender that
    /// can be used to inject events from plugin config providers.
    ///
    /// # Errors
    /// Returns `CoreError` if any provider fails to load initial configs fatally.
    pub async fn start(self) -> Result<(JoinHandle<()>, PluginRoutes), CoreError> {
        let (tx, rx) = mpsc::channel::<ProviderEvent>(256);

        // Phase 1: load initial configs from each provider
        for provider in &self.providers {
            match provider.load_initial().await {
                Ok(configs) => {
                    let count = configs.len();
                    let server_configs: Vec<_> =
                        configs.iter().map(|pc| &pc.config).cloned().collect();
                    if let Err(e) = infrarust_config::validate_server_configs(&server_configs) {
                        tracing::warn!(
                            provider = provider.provider_type(),
                            error = %e,
                            "duplicate server ID detected"
                        );
                    }
                    for pc in configs {
                        if accepted(&pc, &self.default_forwarding) {
                            self.domain_router.add(pc.id, pc.config);
                        }
                    }
                    tracing::info!(
                        provider = provider.provider_type(),
                        count,
                        "provider loaded initial configs"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.provider_type(),
                        error = %e,
                        "provider failed to load initial configs, skipping"
                    );
                }
            }
        }

        // Phase 2: spawn watch tasks
        for provider in self.providers {
            let sender = tx.clone();
            let shutdown = self.shutdown.clone();
            let provider_type = provider.provider_type().to_string();

            tokio::spawn(async move {
                if let Err(e) = provider.watch(sender, shutdown).await {
                    tracing::warn!(
                        provider = %provider_type,
                        error = %e,
                        "provider watch task exited with error"
                    );
                }
            });
        }

        drop(tx);
        let (plugin_routes, plugin_events) = PluginRoutes::channel();

        // Phase 3: spawn the event loop
        let shutdown = self.shutdown;
        let event_loop = EventLoop {
            router: self.domain_router,
            event_bus: self.event_bus,
            routes_listener: self.routes_listener,
            default_forwarding: self.default_forwarding,
        };
        let handle = tokio::spawn(async move {
            event_loop.run(rx, plugin_events, &shutdown).await;
            if !shutdown.is_cancelled() {
                tracing::error!(
                    "provider event loop exited unexpectedly, configuration hot-reload is no longer active"
                );
            }
        });

        Ok((handle, plugin_routes))
    }
}

struct EventLoop {
    router: Arc<DomainRouter>,
    event_bus: Arc<EventBusImpl>,
    routes_listener: Arc<dyn RoutesListener>,
    default_forwarding: ForwardingMode,
}

impl EventLoop {
    async fn run(
        self,
        mut providers: mpsc::Receiver<ProviderEvent>,
        mut plugins: PluginRouteEvents,
        shutdown: &CancellationToken,
    ) {
        loop {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::debug!("provider registry shutting down");
                    break;
                }
                Some(plugin_id) = plugins.unloads.recv() => {
                    self.apply(self.removal_of(&plugin_id)).await;
                }
                Some(submitted) = plugins.changes.recv() => {
                    if submitted.watch.is_cancelled() {
                        continue;
                    }
                    self.apply(submitted.changes).await;
                    if let Some(applied) = submitted.applied {
                        let _ = applied.send(());
                    }
                }
                Some(event) = providers.recv() => {
                    self.apply(event.into_changes()).await;
                }
            }
        }
    }

    fn removal_of(&self, plugin_id: &str) -> Vec<ProviderChange> {
        self.router
            .list_all()
            .into_iter()
            .filter(|(id, _)| id.plugin_owner() == Some(plugin_id))
            .map(|(id, _)| ProviderChange::Removed(id))
            .collect()
    }

    async fn apply(&self, changes: Vec<ProviderChange>) {
        let reloads = apply(&self.router, &self.default_forwarding, changes);
        if reloads.is_empty() {
            return;
        }
        self.routes_listener.routes_changed().await;
        for reload in reloads {
            self.event_bus.post(reload);
        }
    }
}

pub(super) fn accepted(pc: &ProviderConfig, default_forwarding: &ForwardingMode) -> bool {
    match infrarust_config::validate_server_forwarding(&pc.config, default_forwarding.clone()) {
        Ok(warnings) => {
            for warning in warnings {
                tracing::warn!(id = %pc.id, "{warning}");
            }
            true
        }
        Err(error) => {
            tracing::error!(id = %pc.id, %error, "server config rejected");
            false
        }
    }
}

fn apply(
    router: &DomainRouter,
    default_forwarding: &ForwardingMode,
    changes: Vec<ProviderChange>,
) -> Vec<ConfigReloadEvent> {
    let mut before: Vec<(ProviderId, Option<Arc<ServerConfig>>)> = Vec::new();
    for change in changes {
        let id = match &change {
            ProviderChange::Added(pc) | ProviderChange::Updated(pc) => {
                if !accepted(pc, default_forwarding) {
                    continue;
                }
                pc.id.clone()
            }
            ProviderChange::Removed(id) => id.clone(),
        };
        if !before.iter().any(|(seen, _)| *seen == id) {
            let previous = router.get(&id);
            before.push((id, previous));
        }
        let routing = match change {
            ProviderChange::Added(pc) => {
                tracing::info!(id = %pc.id, "config added by provider");
                router.add(pc.id, pc.config)
            }
            ProviderChange::Updated(pc) => {
                tracing::info!(id = %pc.id, "config updated by provider");
                router.update(pc.id, pc.config)
            }
            ProviderChange::Removed(id) => {
                tracing::info!(id = %id, "config removed by provider");
                router.remove(&id)
            }
        };
        for (id, was) in routing.left {
            if !before.iter().any(|(seen, _)| *seen == id) {
                before.push((id, Some(was)));
            }
        }
        for id in routing.entered {
            if !before.iter().any(|(seen, _)| *seen == id) {
                before.push((id, None));
            }
        }
    }

    let mut reloads: Vec<ConfigReloadEvent> = Vec::new();
    for (id, previous) in before {
        let position = reloads
            .iter()
            .position(|reload| reload.provider == id.provider_type)
            .unwrap_or_else(|| {
                reloads.push(ConfigReloadEvent::new(
                    id.provider_type.clone(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ));
                reloads.len() - 1
            });
        let reload = &mut reloads[position];
        match (previous, router.get(&id)) {
            (None, Some(now)) => reload.added.push(ServerId::new(now.effective_id())),
            (Some(was), None) => reload.removed.push(ServerId::new(was.effective_id())),
            (Some(was), Some(now)) if was.effective_id() != now.effective_id() => {
                reload.removed.push(ServerId::new(was.effective_id()));
                reload.added.push(ServerId::new(now.effective_id()));
            }
            (Some(was), Some(now)) if was != now => {
                reload.updated.push(ServerId::new(now.effective_id()));
            }
            _ => {}
        }
    }
    reloads.retain(|reload| !reload.is_empty());
    for reload in &mut reloads {
        for ids in [&mut reload.added, &mut reload.removed, &mut reload.updated] {
            ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            ids.dedup();
        }
    }
    reloads
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn passthrough(name: &str) -> ProviderConfig {
        ProviderConfig {
            id: ProviderId::file(name),
            config: toml::from_str(&format!(
                "name = \"{name}\"\ndomains = [\"{name}.test\"]\naddresses = [\"10.0.0.1:25565\"]\n"
            ))
            .unwrap(),
        }
    }

    #[test]
    fn a_forwarding_server_under_a_velocity_default_is_not_published() {
        let router = DomainRouter::new();
        let lobby = passthrough("lobby");
        let id = lobby.id.clone();

        let reloads = apply(
            &router,
            &ForwardingMode::Velocity,
            vec![ProviderChange::Added(lobby)],
        );

        assert!(
            router.get(&id).is_none(),
            "the rejected server must not be routed"
        );
        assert!(reloads.is_empty());
    }

    #[test]
    fn a_forwarding_server_inheriting_a_bungee_default_is_not_published() {
        for default in [ForwardingMode::BungeeCord, ForwardingMode::BungeeGuard] {
            let router = DomainRouter::new();
            let lobby = passthrough("lobby");
            let id = lobby.id.clone();

            let reloads = apply(&router, &default, vec![ProviderChange::Added(lobby)]);

            assert!(router.get(&id).is_none(), "{default:?}");
            assert!(reloads.is_empty(), "{default:?}");
        }
    }

    #[test]
    fn a_rejected_update_keeps_the_previous_config() {
        let router = DomainRouter::new();
        let lobby = passthrough("lobby");
        let id = lobby.id.clone();
        apply(
            &router,
            &ForwardingMode::None,
            vec![ProviderChange::Added(lobby)],
        );

        let mut broken = passthrough("lobby");
        broken.config.forwarding_mode = Some(ForwardingMode::Velocity);
        apply(
            &router,
            &ForwardingMode::None,
            vec![ProviderChange::Updated(broken)],
        );

        let kept = router.get(&id).expect("the previous config stays routed");
        assert_eq!(kept.forwarding_mode, None);
    }

    fn lobby_from(id: ProviderId) -> ProviderConfig {
        ProviderConfig {
            id,
            config: toml::from_str(
                "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"10.0.0.1:25565\"]\n",
            )
            .unwrap(),
        }
    }

    #[test]
    fn a_held_config_is_reported_when_it_leaves_and_reenters_routing() {
        let router = DomainRouter::new();
        let file = ProviderId::file("lobby.toml");
        let docker = ProviderId::docker("lobby");
        apply(
            &router,
            &ForwardingMode::None,
            vec![ProviderChange::Added(lobby_from(docker.clone()))],
        );

        let reloads = apply(
            &router,
            &ForwardingMode::None,
            vec![ProviderChange::Added(lobby_from(file.clone()))],
        );
        let docker_reload = reloads.iter().find(|r| r.provider == "docker").unwrap();
        assert_eq!(docker_reload.removed, vec![ServerId::new("lobby")]);
        assert!(router.get(&docker).is_none());

        let reloads = apply(
            &router,
            &ForwardingMode::None,
            vec![ProviderChange::Removed(file)],
        );
        let docker_reload = reloads.iter().find(|r| r.provider == "docker").unwrap();
        assert_eq!(docker_reload.added, vec![ServerId::new("lobby")]);
        assert!(router.get(&docker).is_some());
    }
}
