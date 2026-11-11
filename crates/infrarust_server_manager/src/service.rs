use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use dashmap::mapref::one::RefMut;
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

const MIN_START_INTERVAL: Duration = Duration::from_secs(30);
const FIRST_CRASH_BACKOFF: Duration = Duration::from_secs(30);
const MAX_CRASH_BACKOFF: Duration = Duration::from_secs(10 * 60);
const STABLE_UPTIME: Duration = MAX_CRASH_BACKOFF;

/// Limits how often player logins can start a server.
pub(crate) struct StartThrottle {
    last_start: Option<Instant>,
    blocked_until: Option<Instant>,
    next_backoff: Duration,
    stop_requested: bool,
}

impl StartThrottle {
    const fn new() -> Self {
        Self {
            last_start: None,
            blocked_until: None,
            next_backoff: FIRST_CRASH_BACKOFF,
            stop_requested: false,
        }
    }

    fn started(&mut self, now: Instant) {
        self.last_start = Some(now);
        self.blocked_until = None;
        self.stop_requested = false;
    }

    const fn stop_requested(&mut self) {
        self.stop_requested = true;
    }

    fn crashed(&mut self, now: Instant) {
        let stable = self
            .last_start
            .is_none_or(|start| now.saturating_duration_since(start) >= STABLE_UPTIME);
        if stable {
            self.next_backoff = FIRST_CRASH_BACKOFF;
        }
        self.blocked_until = Some(now + self.next_backoff);
        self.next_backoff = (self.next_backoff * 2).min(MAX_CRASH_BACKOFF);
    }

    fn exited(&mut self, now: Instant) {
        if std::mem::take(&mut self.stop_requested) {
            return;
        }
        let unstable = self
            .last_start
            .is_some_and(|start| now.saturating_duration_since(start) < STABLE_UPTIME);
        if unstable {
            self.crashed(now);
        } else {
            self.next_backoff = FIRST_CRASH_BACKOFF;
        }
    }

    fn running(&mut self) {
        self.blocked_until = None;
    }

    fn retry_after(&self, now: Instant) -> Option<Duration> {
        let next_start = self.last_start.map(|start| start + MIN_START_INTERVAL);
        next_start
            .max(self.blocked_until)?
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
    }
}

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
    /// Set by a start request until the provider reports it starting or running:
    /// before this deadline a `Stopped` poll means the panel has not caught up yet.
    pub(crate) unconfirmed_start_until: Option<Instant>,
    throttle: StartThrottle,
}

impl ServerEntry {
    /// Sole mutation point for `state`: keeps the generation in sync.
    pub(crate) fn set_state(&mut self, new_state: ServerState) {
        if new_state == ServerState::Crashed && self.state != ServerState::Crashed {
            self.throttle.crashed(Instant::now());
        }
        self.generation += 1;
        self.state = new_state;
        self.unconfirmed_start_until = None;
    }

    pub(crate) fn begin_start(&mut self) {
        self.set_state(ServerState::Starting);
        let now = Instant::now();
        self.throttle.started(now);
        self.unconfirmed_start_until = Some(now + self.start_timeout);
    }

    fn is_unconfirmed_start(&self) -> bool {
        self.unconfirmed_start_until
            .is_some_and(|deadline| Instant::now() < deadline)
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
                    cfg.poll_interval,
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
                    unconfirmed_start_until: None,
                    throttle: StartThrottle::new(),
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
                unconfirmed_start_until: None,
                throttle: StartThrottle::new(),
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
        let mut listeners = self
            .listeners
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        listeners.push((id, callback));
        id
    }

    fn fire_state_change(&self, server_id: &str, old: ServerState, new: ServerState) {
        let snapshot = {
            let listeners = self
                .listeners
                .read()
                .unwrap_or_else(PoisonError::into_inner);
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
    /// - `Sleeping`/`Crashed` → triggers start unless throttled, then waits for Online
    /// - `Starting` → waits for Online (joins existing waiters)
    /// - `Stopping` → returns an error
    ///
    /// # Errors
    ///
    /// Returns [`ServerManagerError::ServerNotFound`] if the server ID is unknown,
    /// [`ServerManagerError::InvalidState`] if the server is stopping,
    /// [`ServerManagerError::StartThrottled`] if it was started or crashed too recently,
    /// [`ServerManagerError::StartTimeout`] if the server doesn't start in time,
    /// or [`ServerManagerError::Provider`] if the provider fails.
    pub async fn ensure_started(
        self: &Arc<Self>,
        server_id: &str,
    ) -> Result<(), ServerManagerError> {
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
                ServerState::Starting => {
                    let (tx, rx) = oneshot::channel();
                    let start_timeout = entry.start_timeout;
                    entry.waiters.retain(|waiter| !waiter.is_closed());
                    entry.waiters.push(tx);
                    (rx, start_timeout)
                }
                ServerState::Sleeping | ServerState::Crashed | ServerState::Unknown => {
                    let start_timeout = entry.start_timeout;
                    let (tx, rx) = oneshot::channel();
                    drop(self.start_unless_throttled(entry, server_id, Some(tx))?);
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

    /// Starts a server for a player who is not kept waiting on this call.
    ///
    /// Obeys the same throttle as [`ensure_started`](Self::ensure_started). A server
    /// that is already online or starting is left as it is.
    ///
    /// # Errors
    ///
    /// Returns [`ServerManagerError::ServerNotFound`] if the server ID is unknown,
    /// [`ServerManagerError::InvalidState`] if the server is stopping,
    /// [`ServerManagerError::StartThrottled`] if it was started or crashed too recently,
    /// or a provider-specific error if the start command fails.
    pub async fn wake(self: &Arc<Self>, server_id: &str) -> Result<(), ServerManagerError> {
        let entry =
            self.entries
                .get_mut(server_id)
                .ok_or_else(|| ServerManagerError::ServerNotFound {
                    server_id: server_id.to_string(),
                })?;
        let start = match entry.state {
            ServerState::Online | ServerState::Starting => return Ok(()),
            ServerState::Stopping => {
                return Err(ServerManagerError::InvalidState {
                    server_id: server_id.to_string(),
                    state: ServerState::Stopping,
                    action: "start".to_string(),
                });
            }
            ServerState::Sleeping | ServerState::Crashed | ServerState::Unknown => {
                self.start_unless_throttled(entry, server_id, None)?
            }
        };
        start.await.unwrap_or_else(|e| {
            Err(ServerManagerError::Provider {
                server_id: server_id.to_string(),
                message: format!("start task failed: {e}"),
            })
        })
    }

    fn start_unless_throttled(
        self: &Arc<Self>,
        mut entry: RefMut<'_, String, ServerEntry>,
        server_id: &str,
        waiter: Option<oneshot::Sender<Result<(), ServerManagerError>>>,
    ) -> Result<JoinHandle<Result<(), ServerManagerError>>, ServerManagerError> {
        if let Some(retry_after) = entry.throttle.retry_after(Instant::now()) {
            return Err(ServerManagerError::StartThrottled {
                server_id: server_id.to_string(),
                retry_after,
            });
        }
        let provider = Arc::clone(&entry.provider);
        let old_state = entry.state;
        entry.begin_start();
        let attempt = entry.generation;
        entry.waiters.extend(waiter);
        drop(entry);

        self.fire_state_change(server_id, old_state, ServerState::Starting);

        let service = Arc::clone(self);
        let server_id = server_id.to_string();
        Ok(tokio::spawn(async move {
            let started = provider.start().await;
            if let Err(e) = &started {
                service.record_failed_start(&server_id, attempt, e.clone());
            }
            started
        }))
    }

    fn record_failed_start(&self, server_id: &str, attempt: u64, error: ServerManagerError) {
        tracing::error!(server = %server_id, "provider start failed: {error}");
        let Some(mut entry) = self.entries.get_mut(server_id) else {
            return;
        };
        if entry.generation != attempt {
            return;
        }
        entry.set_state(ServerState::Crashed);
        drop(entry);
        self.fire_state_change(server_id, ServerState::Starting, ServerState::Crashed);
        self.notify_waiters(server_id, &Err(error));
    }

    /// Starts a server manually, regardless of the throttle on player logins.
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
            entry.begin_start();
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
            entry.throttle.stop_requested();
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

            match new_status {
                ProviderStatus::Running => {
                    entry.unconfirmed_start_until = None;
                    entry.throttle.running();
                }
                ProviderStatus::Starting => entry.unconfirmed_start_until = None,
                ProviderStatus::Stopped if entry.is_unconfirmed_start() => return None,
                _ if entry.state == ServerState::Crashed => return None,
                _ => {}
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

            if new_status == ProviderStatus::Stopped
                && matches!(old_state, ServerState::Starting | ServerState::Online)
            {
                entry.throttle.exited(Instant::now());
            }
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
                let _ = tx.send(result.clone());
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
            .map_or_else(infrarust_config::defaults::poll_interval, |e| {
                e.poll_interval
            })
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

    fn blocked_for(service: &ServerManagerService, server_id: &str) -> Duration {
        let entry = service.entries.get(server_id).unwrap();
        entry
            .throttle
            .retry_after(Instant::now())
            .unwrap_or_default()
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

    #[tokio::test]
    async fn crashed_only_ends_when_the_provider_reports_it_alive() {
        let service = Arc::new(service_with_online_server("s"));

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Crashed, generation);
        for status in [
            ProviderStatus::Stopped,
            ProviderStatus::Stopping,
            ProviderStatus::Unknown,
            ProviderStatus::Crashed,
        ] {
            let generation = service.generation("s").unwrap();
            assert_eq!(service.update_state("s", status, generation), None);
        }
        assert_eq!(service.get_state("s"), Some(ServerState::Crashed));

        let generation = service.generation("s").unwrap();
        assert_eq!(
            service.update_state("s", ProviderStatus::Running, generation),
            Some((ServerState::Crashed, ServerState::Online))
        );
        service.ensure_started("s").await.unwrap();
    }

    #[tokio::test]
    async fn recovering_from_a_crash_lifts_the_crash_backoff() {
        let service = service_with_online_server("s");

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Crashed, generation);
        assert!(blocked_for(&service, "s") > Duration::ZERO);

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Running, generation);
        assert_eq!(blocked_for(&service, "s"), Duration::ZERO);
    }

    fn rewind_last_start(service: &ServerManagerService, server_id: &str, by: Duration) {
        let mut entry = service.entries.get_mut(server_id).unwrap();
        entry.throttle.last_start = Some(Instant::now() - by);
    }

    fn boot_and_fall_back_to_stopped(service: &ServerManagerService, server_id: &str) {
        let mut entry = service.entries.get_mut(server_id).unwrap();
        entry.begin_start();
        drop(entry);
        rewind_last_start(
            service,
            server_id,
            MIN_START_INTERVAL + Duration::from_secs(1),
        );

        let generation = service.generation(server_id).unwrap();
        service.update_state(server_id, ProviderStatus::Starting, generation);
        let applied = service.update_state(server_id, ProviderStatus::Stopped, generation);
        assert_eq!(
            applied,
            Some((ServerState::Starting, ServerState::Sleeping))
        );
    }

    #[tokio::test]
    async fn a_boot_that_falls_back_to_stopped_backs_off() {
        let service = service_with_online_server("s");

        boot_and_fall_back_to_stopped(&service, "s");
        let first = blocked_for(&service, "s");
        assert!(
            first > Duration::ZERO && first <= FIRST_CRASH_BACKOFF,
            "{first:?}"
        );

        boot_and_fall_back_to_stopped(&service, "s");
        let second = blocked_for(&service, "s");
        assert!(second > FIRST_CRASH_BACKOFF, "{second:?}");
    }

    #[tokio::test]
    async fn a_server_dying_soon_after_its_start_backs_off() {
        let service = service_with_online_server("s");
        service.entries.get_mut("s").unwrap().begin_start();
        rewind_last_start(&service, "s", MIN_START_INTERVAL + Duration::from_secs(1));

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Running, generation);
        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Stopped, generation);

        assert!(blocked_for(&service, "s") > Duration::ZERO);
    }

    #[tokio::test]
    async fn a_stable_server_seen_stopped_is_not_blocked() {
        let service = service_with_online_server("s");
        service.entries.get_mut("s").unwrap().begin_start();

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Running, generation);
        rewind_last_start(&service, "s", STABLE_UPTIME + Duration::from_secs(1));
        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Stopped, generation);

        assert_eq!(blocked_for(&service, "s"), Duration::ZERO);
    }

    #[tokio::test]
    async fn a_stopped_poll_of_a_server_nobody_started_is_not_blocked() {
        let service = service_with_online_server("s");

        let generation = service.generation("s").unwrap();
        service.update_state("s", ProviderStatus::Stopped, generation);

        assert_eq!(blocked_for(&service, "s"), Duration::ZERO);
    }

    #[tokio::test]
    async fn stopped_poll_during_unconfirmed_start_keeps_waiting() {
        let service = service_with_online_server("s");
        service.stop_server("s").await.unwrap();
        service.start_server("s").await.unwrap();

        let generation = service.generation("s").unwrap();
        assert_eq!(
            service.update_state("s", ProviderStatus::Stopped, generation),
            None
        );
        assert_eq!(service.get_state("s"), Some(ServerState::Starting));

        assert_eq!(
            service.update_state("s", ProviderStatus::Starting, generation),
            None
        );
        assert_eq!(
            service.update_state("s", ProviderStatus::Stopped, generation),
            Some((ServerState::Starting, ServerState::Sleeping))
        );
    }

    #[tokio::test]
    async fn stopped_poll_after_start_deadline_goes_to_sleep() {
        let service = service_with_online_server("s");
        service.stop_server("s").await.unwrap();
        service.start_server("s").await.unwrap();
        service
            .entries
            .get_mut("s")
            .unwrap()
            .unconfirmed_start_until = Some(Instant::now());

        let generation = service.generation("s").unwrap();
        assert_eq!(
            service.update_state("s", ProviderStatus::Stopped, generation),
            Some((ServerState::Starting, ServerState::Sleeping))
        );
    }

    #[test]
    fn a_server_never_started_is_not_throttled() {
        assert_eq!(StartThrottle::new().retry_after(Instant::now()), None);
    }

    #[test]
    fn two_starts_are_kept_apart_by_the_minimum_interval() {
        let now = Instant::now();
        let mut throttle = StartThrottle::new();
        throttle.started(now);

        let later = now + Duration::from_secs(10);
        assert_eq!(throttle.retry_after(later), Some(Duration::from_secs(20)));
        assert_eq!(throttle.retry_after(now + MIN_START_INTERVAL), None);
    }

    #[test]
    fn crash_backoff_doubles_up_to_the_maximum() {
        let mut now = Instant::now();
        let mut throttle = StartThrottle::new();

        for expected in [30, 60, 120, 240, 480, 600, 600] {
            throttle.started(now);
            now += Duration::from_secs(1);
            throttle.crashed(now);
            assert_eq!(
                throttle.retry_after(now),
                Some(Duration::from_secs(expected))
            );
            now += Duration::from_secs(expected);
            assert_eq!(throttle.retry_after(now), None);
        }
    }

    #[test]
    fn crash_backoff_starts_over_after_a_stable_run() {
        let mut now = Instant::now();
        let mut throttle = StartThrottle::new();
        for _ in 0..3 {
            throttle.started(now);
            throttle.crashed(now);
        }

        throttle.started(now);
        now += STABLE_UPTIME;
        throttle.crashed(now);

        assert_eq!(throttle.retry_after(now), Some(FIRST_CRASH_BACKOFF));
    }

    #[test]
    fn a_manual_start_clears_the_crash_backoff() {
        let mut now = Instant::now();
        let mut throttle = StartThrottle::new();
        for _ in 0..3 {
            throttle.started(now);
            throttle.crashed(now);
        }

        now += Duration::from_secs(1);
        throttle.started(now);

        assert_eq!(
            throttle.retry_after(now + MIN_START_INTERVAL),
            None,
            "only the minimum interval applies once the operator restarted the server"
        );
    }

    #[tokio::test]
    async fn only_entering_the_crashed_state_counts_as_a_crash() {
        let service = service_with_online_server("s");
        let mut entry = service.entries.get_mut("s").unwrap();
        entry.set_state(ServerState::Crashed);
        let backoff = entry.throttle.next_backoff;

        entry.set_state(ServerState::Crashed);

        assert_eq!(entry.throttle.next_backoff, backoff);
    }
}
