use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_config::ServerManagerConfig;

use crate::crafty::CraftyProvider;
use crate::error::ServerManagerError;
use crate::local::LocalProvider;
use crate::provider::{ProviderStatus, ServerProvider};
use crate::pterodactyl::PterodactylProvider;
use crate::state::ServerState;

/// Trait for counting players on a server.
///
/// Implemented by the connection registry in `infrarust-core` to avoid
/// a circular dependency.
pub trait PlayerCounter: Send + Sync {
    /// Returns the number of players connected to the given server.
    fn count_by_server(&self, server_id: &str) -> usize;
}

/// Callback type for server state change notifications.
pub type StateChangeCallback = Arc<dyn Fn(&str, ServerState, ServerState) + Send + Sync>;

/// Central orchestrator for server management.
///
/// Tracks the state of each managed server, provides wake-up with waiters,
/// adaptive polling, and auto-shutdown after idle.
pub struct ServerManagerService {
    pub(crate) entries: DashMap<String, ServerEntry>,
    listeners: std::sync::RwLock<Vec<(u64, StateChangeCallback)>>,
    next_listener_id: AtomicU64,
}

pub(crate) struct ServerEntry {
    /// Current state.
    pub(crate) state: ServerState,
    /// Transition counter: bumped on every state write so an in-flight monitor
    /// poll can detect that a transition intervened and drop its stale result.
    pub(crate) generation: u64,
    /// The provider controlling this server.
    pub(crate) provider: Arc<dyn ServerProvider>,
    /// Duration of inactivity before auto-shutdown (None = disabled).
    pub(crate) shutdown_after: Option<Duration>,
    /// Maximum time to wait for the server to become Online.
    pub(crate) start_timeout: Duration,
    /// Base polling interval.
    pub(crate) poll_interval: Duration,
    /// Timestamp of the last player seen connected (for auto-shutdown).
    pub(crate) last_player_seen: Option<Instant>,
    /// Waiters: connections waiting for the server to become Online.
    pub(crate) waiters: Vec<oneshot::Sender<Result<(), ServerManagerError>>>,
}

impl ServerEntry {
    /// Sole mutation point for `state`: keeps the generation in sync.
    pub(crate) fn set_state(&mut self, new_state: ServerState) {
        self.generation += 1;
        self.state = new_state;
    }
}

impl ServerManagerService {
    /// Creates the service from server configs.
    ///
    /// For each `(server_id, ServerManagerConfig)`, creates the appropriate provider
    /// and registers it with default state `Sleeping`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        server_configs: &[(String, ServerManagerConfig)],
        http_client: reqwest::Client,
    ) -> Self {
        let entries = DashMap::new();

        for (server_id, config) in server_configs {
            let (provider, shutdown_after, start_timeout, poll_interval): (
                Arc<dyn ServerProvider>,
                Option<Duration>,
                Duration,
                Duration,
            ) = match config {
                ServerManagerConfig::Local(cfg) => (
                    Arc::new(LocalProvider::new(cfg.clone())),
                    cfg.shutdown_after,
                    cfg.start_timeout,
                    Duration::from_secs(5),
                ),
                ServerManagerConfig::Pterodactyl(cfg) => (
                    Arc::new(PterodactylProvider::new(cfg, http_client.clone())),
                    cfg.shutdown_after,
                    cfg.start_timeout,
                    cfg.poll_interval,
                ),
                ServerManagerConfig::Crafty(cfg) => (
                    Arc::new(CraftyProvider::new(cfg, http_client.clone())),
                    cfg.shutdown_after,
                    cfg.start_timeout,
                    cfg.poll_interval,
                ),
            };

            entries.insert(
                server_id.clone(),
                ServerEntry {
                    state: ServerState::Sleeping,
                    generation: 0,
                    provider,
                    shutdown_after,
                    start_timeout,
                    poll_interval,
                    last_player_seen: None,
                    waiters: Vec::new(),
                },
            );

            tracing::info!(server = %server_id, "registered managed server");
        }

        Self {
            entries,
            listeners: std::sync::RwLock::new(Vec::new()),
            next_listener_id: AtomicU64::new(1),
        }
    }

    /// Registers a server with a custom provider.
    pub fn register_server(
        &self,
        server_id: String,
        provider: Arc<dyn ServerProvider>,
        shutdown_after: Option<Duration>,
        start_timeout: Duration,
        poll_interval: Duration,
    ) {
        self.entries.insert(
            server_id,
            ServerEntry {
                state: ServerState::Sleeping,
                generation: 0,
                provider,
                shutdown_after,
                start_timeout,
                poll_interval,
                last_player_seen: None,
                waiters: Vec::new(),
            },
        );
    }

    /// Returns the current state of a server.
    pub fn get_state(&self, server_id: &str) -> Option<ServerState> {
        self.entries.get(server_id).map(|e| e.state)
    }

    /// Lists all managed servers and their states.
    pub fn get_all_managed(&self) -> Vec<(String, ServerState)> {
        self.entries
            .iter()
            .map(|e| (e.key().clone(), e.state))
            .collect()
    }

    /// Registers a callback that fires on every state transition.
    ///
    /// The callback receives `(server_id, old_state, new_state)`.
    pub fn add_on_state_change(&self, callback: StateChangeCallback) -> u64 {
        let id = self.next_listener_id.fetch_add(1, Ordering::Relaxed);
        let mut listeners = self.listeners.write().expect("lock poisoned");
        listeners.push((id, callback));
        id
    }

    /// Removes a previously registered state change listener.
    pub fn remove_on_state_change(&self, listener_id: u64) {
        let mut listeners = self.listeners.write().expect("lock poisoned");
        listeners.retain(|(id, _)| *id != listener_id);
    }

    fn fire_state_change(&self, server_id: &str, old: ServerState, new: ServerState) {
        let snapshot = {
            let listeners = self.listeners.read().expect("lock poisoned");
            listeners.clone()
        };
        for (_, callback) in &snapshot {
            if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback(server_id, old, new);
            })) {
                tracing::error!(
                    server = %server_id,
                    "state change callback panicked: {e:?}"
                );
            }
        }
    }

    /// Performs an initial health check for all managed servers.
    ///
    /// Calls `provider.check_status()` on each server to determine its initial state.
    /// Called once at proxy startup.
    pub async fn initial_health_check(&self) {
        let server_ids: Vec<String> = self.entries.iter().map(|e| e.key().clone()).collect();

        for server_id in server_ids {
            let provider = {
                let Some(entry) = self.entries.get(&server_id) else {
                    continue;
                };
                Arc::clone(&entry.provider)
            };

            match provider.check_status().await {
                Ok(status) => {
                    let new_state = ServerState::from(status);
                    let old_state = {
                        let Some(mut entry) = self.entries.get_mut(&server_id) else {
                            continue;
                        };
                        let old = entry.state;
                        entry.set_state(new_state);
                        old
                    };
                    if old_state != new_state {
                        self.fire_state_change(&server_id, old_state, new_state);
                    }
                    tracing::info!(
                        server = %server_id,
                        state = %new_state,
                        "initial health check complete"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        server = %server_id,
                        error = %e,
                        "initial health check failed, defaulting to Sleeping"
                    );
                }
            }
        }
    }

    /// Ensures the server is started and ready for connections.
    ///
    /// - `Online` → returns Ok immediately
    /// - `Sleeping`/`Crashed` → triggers start, then waits for Online
    /// - `Starting` → waits for Online (joins existing waiters)
    /// - `Stopping` → returns an error
    ///
    /// # Errors
    ///
    /// Returns [`ServerManagerError::ServerNotFound`] if the server ID is unknown,
    /// [`ServerManagerError::InvalidState`] if the server is stopping,
    /// [`ServerManagerError::StartTimeout`] if the server doesn't start in time,
    /// or [`ServerManagerError::Provider`] if the provider fails.
    pub async fn ensure_started(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let (rx, start_timeout) = {
            let mut entry = self.entries.get_mut(server_id).ok_or_else(|| {
                ServerManagerError::ServerNotFound {
                    server_id: server_id.to_string(),
                }
            })?;

            match entry.state {
                ServerState::Online => return Ok(()),
                ServerState::Stopping => {
                    return Err(ServerManagerError::InvalidState {
                        server_id: server_id.to_string(),
                        state: ServerState::Stopping,
                        action: "connect".to_string(),
                    });
                }
                ServerState::Sleeping | ServerState::Crashed => {
                    // Need to start the server
                    let provider = Arc::clone(&entry.provider);
                    let old_state = entry.state;
                    entry.set_state(ServerState::Starting);

                    // Drop the lock before calling provider.start()
                    let start_timeout = entry.start_timeout;
                    let (tx, rx) = oneshot::channel();
                    entry.waiters.push(tx);
                    drop(entry);

                    self.fire_state_change(server_id, old_state, ServerState::Starting);

                    // Call start on the provider (lock-free)
                    if let Err(e) = provider.start().await {
                        tracing::error!(server = %server_id, "provider start failed: {e}");
                        // Reset state
                        if let Some(mut entry) = self.entries.get_mut(server_id) {
                            entry.set_state(ServerState::Crashed);
                            // Notify waiters of failure
                            let waiters = std::mem::take(&mut entry.waiters);
                            drop(entry);
                            self.fire_state_change(
                                server_id,
                                ServerState::Starting,
                                ServerState::Crashed,
                            );
                            for tx in waiters {
                                let _ = tx.send(Err(ServerManagerError::Provider {
                                    server_id: server_id.to_string(),
                                    message: "start failed".to_string(),
                                }));
                            }
                        }
                        return Err(e);
                    }

                    (rx, start_timeout)
                }
                ServerState::Starting => {
                    // Already starting — just add a waiter
                    let (tx, rx) = oneshot::channel();
                    let start_timeout = entry.start_timeout;
                    entry.waiters.push(tx);
                    (rx, start_timeout)
                }
                _ => {
                    // Unknown — try to start
                    let provider = Arc::clone(&entry.provider);
                    let old_state = entry.state;
                    entry.set_state(ServerState::Starting);
                    let start_timeout = entry.start_timeout;
                    let (tx, rx) = oneshot::channel();
                    entry.waiters.push(tx);
                    drop(entry);

                    self.fire_state_change(server_id, old_state, ServerState::Starting);
                    if let Err(e) = provider.start().await {
                        tracing::error!(server = %server_id, "provider start failed: {e}");
                        if let Some(mut entry) = self.entries.get_mut(server_id) {
                            entry.set_state(ServerState::Crashed);
                            let waiters = std::mem::take(&mut entry.waiters);
                            drop(entry);
                            self.fire_state_change(
                                server_id,
                                ServerState::Starting,
                                ServerState::Crashed,
                            );
                            for tx in waiters {
                                let _ = tx.send(Err(ServerManagerError::Provider {
                                    server_id: server_id.to_string(),
                                    message: "start failed".to_string(),
                                }));
                            }
                        }
                        return Err(e);
                    }
                    (rx, start_timeout)
                }
            }
        };

        // Wait for the server to become Online (or timeout)
        match tokio::time::timeout(start_timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // Channel closed without a message — monitoring task died
                Err(ServerManagerError::Provider {
                    server_id: server_id.to_string(),
                    message: "waiter channel closed unexpectedly".to_string(),
                })
            }
            Err(_) => Err(ServerManagerError::StartTimeout {
                server_id: server_id.to_string(),
                timeout: start_timeout,
            }),
        }
    }

    /// Starts a server manually.
    ///
    /// # Errors
    ///
    /// Returns [`ServerManagerError::ServerNotFound`] if the server ID is unknown,
    /// [`ServerManagerError::InvalidState`] if the server is not in a startable state,
    /// or a provider-specific error if the start command fails.
    pub async fn start_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let (provider, old_state) = {
            let mut entry = self.entries.get_mut(server_id).ok_or_else(|| {
                ServerManagerError::ServerNotFound {
                    server_id: server_id.to_string(),
                }
            })?;

            if !entry.state.is_startable() {
                return Err(ServerManagerError::InvalidState {
                    server_id: server_id.to_string(),
                    state: entry.state,
                    action: "start".to_string(),
                });
            }

            let old = entry.state;
            entry.set_state(ServerState::Starting);
            (Arc::clone(&entry.provider), old)
        };

        self.fire_state_change(server_id, old_state, ServerState::Starting);
        provider.start().await
    }

    /// Stops a server manually.
    ///
    /// # Errors
    ///
    /// Returns [`ServerManagerError::ServerNotFound`] if the server ID is unknown,
    /// or a provider-specific error if the stop command fails.
    pub async fn stop_server(&self, server_id: &str) -> Result<(), ServerManagerError> {
        let (provider, old_state) = {
            let mut entry = self.entries.get_mut(server_id).ok_or_else(|| {
                ServerManagerError::ServerNotFound {
                    server_id: server_id.to_string(),
                }
            })?;

            let old = entry.state;
            entry.set_state(ServerState::Stopping);
            (Arc::clone(&entry.provider), old)
        };

        self.fire_state_change(server_id, old_state, ServerState::Stopping);

        if let Err(e) = provider.stop().await {
            if let Some(mut entry) = self.entries.get_mut(server_id) {
                entry.set_state(old_state);
            }
            self.fire_state_change(server_id, ServerState::Stopping, old_state);
            return Err(e);
        }

        if let Some(mut entry) = self.entries.get_mut(server_id) {
            entry.set_state(ServerState::Sleeping);
            entry.last_player_seen = None;
        }

        self.fire_state_change(server_id, ServerState::Stopping, ServerState::Sleeping);

        Ok(())
    }

    /// Starts monitoring tasks for all managed servers.
    ///
    /// Returns join handles for the spawned tasks.
    #[allow(clippy::needless_pass_by_value)]
    pub fn start_monitoring(
        self: &Arc<Self>,
        player_counter: Arc<dyn PlayerCounter>,
        shutdown: CancellationToken,
    ) -> Vec<JoinHandle<()>> {
        let server_ids: Vec<String> = self.entries.iter().map(|e| e.key().clone()).collect();
        let mut handles = Vec::with_capacity(server_ids.len());

        for server_id in server_ids {
            let service = Arc::clone(self);
            let counter = Arc::clone(&player_counter);
            let token = shutdown.clone();

            let handle = tokio::spawn(async move {
                crate::monitor::monitor_server(service, server_id, counter, token).await;
            });

            handles.push(handle);
        }

        handles
    }

    /// Checks provider status for a server (used by monitoring task).
    pub(crate) async fn check_provider_status(
        &self,
        server_id: &str,
    ) -> Result<ProviderStatus, ServerManagerError> {
        let provider = {
            let entry =
                self.entries
                    .get(server_id)
                    .ok_or_else(|| ServerManagerError::ServerNotFound {
                        server_id: server_id.to_string(),
                    })?;
            Arc::clone(&entry.provider)
        };

        provider.check_status().await
    }

    /// Applies a polled status and returns the previous state if changed.
    ///
    /// `observed_generation` must be captured **before** the poll; if a
    /// transition bumped the generation while the poll was in flight, the
    /// stale result is dropped so manual operations always win.
    pub(crate) fn update_state(
        &self,
        server_id: &str,
        new_status: ProviderStatus,
        observed_generation: u64,
    ) -> Option<(ServerState, ServerState)> {
        let (old_state, new_state) = {
            let mut entry = self.entries.get_mut(server_id)?;

            if entry.generation != observed_generation {
                tracing::debug!(
                    server = %server_id,
                    status = ?new_status,
                    "dropping stale poll result: a transition intervened during the poll"
                );
                return None;
            }

            let new_state = ServerState::from(new_status);
            let old_state = entry.state;

            if old_state == new_state {
                return None;
            }

            tracing::info!(
                server = %server_id,
                from = %old_state,
                to = %new_state,
                "state transition"
            );

            entry.set_state(new_state);
            drop(entry);
            (old_state, new_state)
        };

        // Fire callback outside DashMap lock
        self.fire_state_change(server_id, old_state, new_state);

        Some((old_state, new_state))
    }

    /// Notifies all waiters for a server with the given result.
    pub(crate) fn notify_waiters(&self, server_id: &str, result: &Result<(), ServerManagerError>) {
        if let Some(mut entry) = self.entries.get_mut(server_id) {
            let waiters = std::mem::take(&mut entry.waiters);
            drop(entry);

            let count = waiters.len();
            if count > 0 {
                tracing::debug!(server = %server_id, count, "notifying waiters");
            }
            for tx in waiters {
                let _ = match result {
                    Ok(()) => tx.send(Ok(())),
                    Err(_) => tx.send(Err(ServerManagerError::Provider {
                        server_id: server_id.to_string(),
                        message: "server failed to start".to_string(),
                    })),
                };
            }
        }
    }

    /// Returns the current transition generation for a server.
    pub(crate) fn generation(&self, server_id: &str) -> Option<u64> {
        self.entries.get(server_id).map(|e| e.generation)
    }

    /// Returns the poll interval for a server.
    pub(crate) fn get_poll_interval(&self, server_id: &str) -> Duration {
        self.entries
            .get(server_id)
            .map_or(Duration::from_secs(5), |e| e.poll_interval)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::*;

    struct NoopProvider;

    impl ServerProvider for NoopProvider {
        fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send>> {
            Box::pin(async { Ok(()) })
        }

        fn stop(&self) -> Pin<Box<dyn Future<Output = Result<(), ServerManagerError>> + Send>> {
            Box::pin(async { Ok(()) })
        }

        fn check_status(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<ProviderStatus, ServerManagerError>> + Send>>
        {
            Box::pin(async { Ok(ProviderStatus::Running) })
        }

        fn provider_type(&self) -> &'static str {
            "noop"
        }
    }

    fn service_with_online_server(server_id: &str) -> ServerManagerService {
        let service = ServerManagerService::new(&[], reqwest::Client::new());
        service.register_server(
            server_id.to_string(),
            Arc::new(NoopProvider),
            None,
            Duration::from_secs(5),
            Duration::from_secs(1),
        );
        let generation = service.generation(server_id).unwrap();
        service.update_state(server_id, ProviderStatus::Running, generation);
        assert_eq!(service.get_state(server_id), Some(ServerState::Online));
        service
    }

    /// A poll captured before `stop_server` must not resurrect the server,
    /// even when the manual op fully completes (Stopping → Sleeping) before
    /// the stale result lands.
    #[tokio::test]
    async fn stale_poll_does_not_resurrect_manually_stopped_server() {
        let service = service_with_online_server("s");

        let observed = service.generation("s").unwrap();
        service.stop_server("s").await.unwrap();
        assert_eq!(service.get_state("s"), Some(ServerState::Sleeping));

        let applied = service.update_state("s", ProviderStatus::Running, observed);
        assert_eq!(applied, None);
        assert_eq!(service.get_state("s"), Some(ServerState::Sleeping));
    }

    #[tokio::test]
    async fn online_to_sleeping_on_clean_stop_poll() {
        let service = service_with_online_server("s");

        let generation = service.generation("s").unwrap();
        let applied = service.update_state("s", ProviderStatus::Stopped, generation);
        assert_eq!(applied, Some((ServerState::Online, ServerState::Sleeping)));
    }

    #[tokio::test]
    async fn online_to_crashed_on_unexpected_exit_poll() {
        let service = service_with_online_server("s");

        let generation = service.generation("s").unwrap();
        let applied = service.update_state("s", ProviderStatus::Crashed, generation);
        assert_eq!(applied, Some((ServerState::Online, ServerState::Crashed)));
    }
}
