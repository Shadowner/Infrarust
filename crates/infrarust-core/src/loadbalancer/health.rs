//! Backend health view consumed by the load balancer.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use dashmap::{DashMap, DashSet};

use infrarust_config::ServerAddress;
use infrarust_transport::{ConnectAttempt, ConnectAttemptObserver};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    Healthy,
    Probing,
    Unhealthy,
    /// Taken out of rotation by an operator. Distinct from `Unhealthy`: a
    /// drained address is not even offered as a last-resort failover.
    Draining,
}

impl BackendState {
    pub const fn to_api(self) -> infrarust_api::services::load_balancer::BackendState {
        use infrarust_api::services::load_balancer::BackendState as Api;
        match self {
            Self::Healthy => Api::Healthy,
            Self::Probing => Api::Probing,
            Self::Unhealthy => Api::Unhealthy,
            Self::Draining => Api::Draining,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HealthSnapshot {
    pub state: BackendState,
    pub healthy_since: Option<Instant>,
}

impl HealthSnapshot {
    const fn stable_healthy() -> Self {
        Self {
            state: BackendState::Healthy,
            healthy_since: None,
        }
    }
}

/// Everything the admin API reports about one address.
#[derive(Debug, Clone, Copy)]
pub struct BackendReport {
    pub state: BackendState,
    pub healthy_since: Option<Instant>,
    pub ejections: u32,
    pub last_failure: Option<Instant>,
}

pub trait BackendHealthView: Send + Sync {
    fn snapshot(&self, addr: &ServerAddress) -> HealthSnapshot;

    fn claim_probe(&self, _addr: &ServerAddress) -> bool {
        false
    }
}

const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

const FAILURE_WINDOW: Duration = Duration::from_secs(10);

const BASE_EJECTION: Duration = Duration::from_secs(30);

const MAX_EJECTION_BACKOFF: Duration = Duration::from_secs(300);

const PRUNE_INTERVAL_OPS: u32 = 1024;

const PRUNE_STABLE_AFTER: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy)]
struct AddressHealth {
    consecutive_failures: u32,
    first_failure: Option<Instant>,
    last_failure: Option<Instant>,
    ejections: u32,
    probe_claimed_at: Option<Instant>,
    healthy: bool,
    healthy_since: Option<Instant>,
}

impl AddressHealth {
    const fn fresh(healthy: bool) -> Self {
        Self {
            consecutive_failures: 0,
            first_failure: None,
            last_failure: None,
            ejections: 0,
            probe_claimed_at: None,
            healthy,
            healthy_since: None,
        }
    }

    fn backoff(&self) -> Duration {
        BASE_EJECTION
            .saturating_mul(self.ejections.max(1))
            .min(MAX_EJECTION_BACKOFF)
    }

    fn backoff_elapsed(&self, now: Instant) -> bool {
        self.last_failure
            .is_some_and(|at| now.duration_since(at) >= self.backoff())
    }

    fn state(&self, now: Instant) -> BackendState {
        if self.healthy {
            BackendState::Healthy
        } else if self.backoff_elapsed(now) {
            BackendState::Probing
        } else {
            BackendState::Unhealthy
        }
    }
}

pub trait HealthTransitionListener: Send + Sync {
    fn on_transition(&self, address: &ServerAddress, to: BackendState);
}

pub struct PassiveBackendHealth {
    state: DashMap<ServerAddress, AddressHealth>,
    /// Operator intent, kept apart from observed health: it must outlive both
    /// pruning and an address leaving every config, which `state` must not.
    drained: DashSet<ServerAddress>,
    failure_threshold: u32,
    inserts_since_prune: AtomicU32,
    listeners: RwLock<Vec<Arc<dyn HealthTransitionListener>>>,
}

impl PassiveBackendHealth {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_FAILURE_THRESHOLD)
    }

    pub fn with_threshold(failure_threshold: u32) -> Self {
        Self {
            state: DashMap::new(),
            drained: DashSet::new(),
            failure_threshold: failure_threshold.max(1),
            inserts_since_prune: AtomicU32::new(0),
            listeners: RwLock::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn with_listener(self, listener: Arc<dyn HealthTransitionListener>) -> Self {
        self.add_listener(listener);
        self
    }

    pub fn add_listener(&self, listener: Arc<dyn HealthTransitionListener>) {
        self.listeners
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(listener);
    }

    fn notify(&self, addr: &ServerAddress, to: BackendState) {
        for listener in self
            .listeners
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            listener.on_transition(addr, to);
        }
    }

    pub fn record_success(&self, addr: &ServerAddress) {
        let recovered = {
            let Some(mut entry) = self.state.get_mut(addr) else {
                return; // never failed → implicitly healthy and stable
            };
            entry.consecutive_failures = 0;
            entry.first_failure = None;
            entry.probe_claimed_at = None;
            if entry.healthy {
                None
            } else {
                let now = Instant::now();
                entry.healthy = true;
                entry.healthy_since = Some(now);
                tracing::info!(address = %addr, "backend address healthy again");
                Some(entry.state(now))
            }
        };
        if let Some(state) = recovered {
            self.notify(addr, state);
        }
    }

    pub fn record_failure(&self, addr: &ServerAddress) {
        let mut ejected = None;
        {
            let now = Instant::now();
            let mut entry = self
                .state
                .entry(addr.clone())
                .or_insert(AddressHealth::fresh(true));

            let was_probing = entry.probe_claimed_at.take().is_some();
            if entry
                .first_failure
                .is_some_and(|at| now.duration_since(at) > FAILURE_WINDOW)
            {
                entry.consecutive_failures = 0;
                entry.first_failure = None;
            }
            entry.first_failure.get_or_insert(now);
            entry.consecutive_failures += 1;
            entry.last_failure = Some(now);

            if entry.healthy && entry.consecutive_failures >= self.failure_threshold {
                if entry
                    .healthy_since
                    .is_none_or(|since| now.duration_since(since) >= MAX_EJECTION_BACKOFF)
                {
                    entry.ejections = 0;
                }
                entry.healthy = false;
                entry.healthy_since = None;
                entry.ejections = entry.ejections.saturating_add(1);
                ejected = Some(entry.state(now));
                tracing::warn!(
                    address = %addr,
                    failures = entry.consecutive_failures,
                    backoff = ?entry.backoff(),
                    "backend address marked unhealthy"
                );
            } else if !entry.healthy && was_probing {
                entry.ejections = entry.ejections.saturating_add(1);
            }
        }
        if let Some(state) = ejected {
            self.notify(addr, state);
        }
        self.maybe_prune();
    }

    /// Takes an address out of rotation, or puts it back. Observed health
    /// keeps being tracked underneath, and is what the address goes back to.
    pub fn set_drained(&self, addr: &ServerAddress, drained: bool) {
        let changed = if drained {
            self.drained.insert(addr.clone())
        } else {
            self.drained.remove(addr).is_some()
        };
        if !changed {
            return;
        }
        tracing::info!(address = %addr, drained, "backend address drain state changed");
        let to = if drained {
            BackendState::Draining
        } else {
            self.observed(addr)
        };
        self.notify(addr, to);
    }

    fn observed(&self, addr: &ServerAddress) -> BackendState {
        self.state
            .get(addr)
            .map_or(BackendState::Healthy, |entry| entry.state(Instant::now()))
    }

    /// Forgets the failure history of an address: it rejoins the pool and
    /// ramps back up through slow start.
    pub fn reset(&self, addr: &ServerAddress) {
        let transition = {
            let Some(mut entry) = self.state.get_mut(addr) else {
                return;
            };
            let now = Instant::now();
            entry.consecutive_failures = 0;
            entry.first_failure = None;
            entry.last_failure = None;
            entry.ejections = 0;
            entry.probe_claimed_at = None;
            if entry.healthy {
                None
            } else {
                entry.healthy = true;
                entry.healthy_since = Some(now);
                Some(entry.state(now))
            }
        };
        if let Some(state) = transition {
            self.notify(addr, state);
        }
    }

    pub fn report(&self, addr: &ServerAddress) -> BackendReport {
        let drained = self.drained.contains(addr);
        let Some(entry) = self.state.get(addr) else {
            return BackendReport {
                state: if drained {
                    BackendState::Draining
                } else {
                    BackendState::Healthy
                },
                healthy_since: None,
                ejections: 0,
                last_failure: None,
            };
        };
        BackendReport {
            state: if drained {
                BackendState::Draining
            } else {
                entry.state(Instant::now())
            },
            healthy_since: entry.healthy.then_some(entry.healthy_since).flatten(),
            ejections: entry.ejections,
            last_failure: entry.last_failure,
        }
    }

    pub fn mark_warming(&self, addr: &ServerAddress, ramp_window: Duration) {
        {
            let mut entry = self
                .state
                .entry(addr.clone())
                .or_insert(AddressHealth::fresh(false));
            entry.consecutive_failures = 0;
            entry.first_failure = None;
            entry.probe_claimed_at = None;
            let in_ramp = entry.healthy
                && entry
                    .healthy_since
                    .is_some_and(|since| since.elapsed() < ramp_window);
            if !in_ramp {
                entry.healthy = true;
                entry.healthy_since = Some(Instant::now());
            }
        }
        self.maybe_prune();
    }

    pub fn retain_known(&self, known: &HashSet<ServerAddress>) {
        self.state.retain(|addr, _| known.contains(addr));
    }

    fn maybe_prune(&self) {
        if self.inserts_since_prune.fetch_add(1, Ordering::Relaxed) < PRUNE_INTERVAL_OPS {
            return;
        }
        self.inserts_since_prune.store(0, Ordering::Relaxed);
        self.state.retain(|_, e| {
            !e.healthy
                || e.consecutive_failures > 0
                || e.healthy_since
                    .is_some_and(|since| since.elapsed() < PRUNE_STABLE_AFTER)
        });
    }

    #[cfg(test)]
    fn tracked_addresses(&self) -> usize {
        self.state.len()
    }

    #[cfg(test)]
    pub(crate) fn rewind_failures_for_test(&self, addr: &ServerAddress) {
        if let Some(mut entry) = self.state.get_mut(addr) {
            let by = MAX_EJECTION_BACKOFF;
            entry.last_failure = entry.last_failure.map(|at| at - by);
            entry.first_failure = entry.first_failure.map(|at| at - by);
            entry.probe_claimed_at = entry.probe_claimed_at.map(|at| at - by);
        }
    }
}

impl Default for PassiveBackendHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendHealthView for PassiveBackendHealth {
    fn snapshot(&self, addr: &ServerAddress) -> HealthSnapshot {
        if self.drained.contains(addr) {
            return HealthSnapshot {
                state: BackendState::Draining,
                healthy_since: None,
            };
        }
        let Some(entry) = self.state.get(addr) else {
            return HealthSnapshot::stable_healthy();
        };
        let state = entry.state(Instant::now());
        HealthSnapshot {
            state,
            healthy_since: match state {
                BackendState::Healthy => entry.healthy_since,
                BackendState::Probing | BackendState::Unhealthy | BackendState::Draining => None,
            },
        }
    }

    fn claim_probe(&self, addr: &ServerAddress) -> bool {
        let Some(mut entry) = self.state.get_mut(addr) else {
            return false;
        };
        let now = Instant::now();
        if entry.healthy || !entry.backoff_elapsed(now) {
            return false;
        }
        let backoff = entry.backoff();
        if entry
            .probe_claimed_at
            .is_some_and(|at| now.duration_since(at) < backoff)
        {
            return false;
        }
        entry.probe_claimed_at = Some(now);
        true
    }
}

impl ConnectAttemptObserver for PassiveBackendHealth {
    fn on_attempt(&self, attempt: &ConnectAttempt<'_>) {
        if attempt.succeeded() {
            self.record_success(attempt.address);
        } else {
            self.record_failure(attempt.address);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    fn state(health: &PassiveBackendHealth, a: &ServerAddress) -> BackendState {
        health.snapshot(a).state
    }

    fn age_failure(health: &PassiveBackendHealth, a: &ServerAddress, by: Duration) {
        let mut entry = health.state.get_mut(a).expect("tracked address");
        entry.last_failure = entry.last_failure.map(|at| at - by);
        entry.first_failure = entry.first_failure.map(|at| at - by);
    }

    #[test]
    fn unknown_address_is_healthy_and_stable() {
        let health = PassiveBackendHealth::new();
        let snap = health.snapshot(&addr("a"));
        assert_eq!(snap.state, BackendState::Healthy);
        assert!(snap.healthy_since.is_none());
    }

    #[test]
    fn unhealthy_after_threshold_failures() {
        let health = PassiveBackendHealth::with_threshold(3);
        let a = addr("a");
        health.record_failure(&a);
        health.record_failure(&a);
        assert_eq!(state(&health, &a), BackendState::Healthy);
        health.record_failure(&a);
        assert_eq!(state(&health, &a), BackendState::Unhealthy);
    }

    #[test]
    fn failures_outside_the_window_do_not_accumulate() {
        let health = PassiveBackendHealth::with_threshold(3);
        let a = addr("a");
        health.record_failure(&a);
        health.record_failure(&a);
        age_failure(&health, &a, FAILURE_WINDOW * 2);
        health.record_failure(&a);
        assert_eq!(
            state(&health, &a),
            BackendState::Healthy,
            "a stale streak must not eject"
        );
    }

    #[test]
    fn success_resets_failure_streak() {
        let health = PassiveBackendHealth::with_threshold(3);
        let a = addr("a");
        health.record_failure(&a);
        health.record_failure(&a);
        health.record_success(&a);
        health.record_failure(&a);
        health.record_failure(&a);
        assert_eq!(state(&health, &a), BackendState::Healthy);
    }

    #[test]
    fn ejected_becomes_probing_after_backoff() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        assert_eq!(state(&health, &a), BackendState::Unhealthy);
        age_failure(&health, &a, BASE_EJECTION);
        assert_eq!(state(&health, &a), BackendState::Probing);
    }

    #[test]
    fn probe_claim_is_exclusive_per_window() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        assert!(!health.claim_probe(&a), "still within backoff");
        age_failure(&health, &a, BASE_EJECTION);
        assert!(health.claim_probe(&a));
        assert!(!health.claim_probe(&a), "one probe per window");
    }

    #[test]
    fn healthy_address_is_never_claimable() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.record_failure(&a);
        assert!(!health.claim_probe(&a));
    }

    #[test]
    fn failed_probe_extends_the_backoff() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        age_failure(&health, &a, BASE_EJECTION);
        assert!(health.claim_probe(&a));
        health.record_failure(&a);

        age_failure(&health, &a, BASE_EJECTION);
        assert_eq!(
            state(&health, &a),
            BackendState::Unhealthy,
            "second ejection must wait twice as long"
        );
        age_failure(&health, &a, BASE_EJECTION);
        assert_eq!(state(&health, &a), BackendState::Probing);
    }

    #[test]
    fn backoff_is_capped() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        for _ in 0..50 {
            age_failure(&health, &a, MAX_EJECTION_BACKOFF);
            assert!(health.claim_probe(&a));
            health.record_failure(&a);
        }
        age_failure(&health, &a, MAX_EJECTION_BACKOFF);
        assert_eq!(state(&health, &a), BackendState::Probing);
    }

    #[test]
    fn successful_probe_restores_health_in_slow_start() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        age_failure(&health, &a, BASE_EJECTION);
        assert!(health.claim_probe(&a));
        health.record_success(&a);

        let snap = health.snapshot(&a);
        assert_eq!(snap.state, BackendState::Healthy);
        assert!(
            snap.healthy_since.is_some(),
            "a reinstated address must ramp back up"
        );
    }

    #[test]
    fn long_stable_address_restarts_its_ejection_count() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        let ejections = || health.state.get(&a).expect("tracked").ejections;

        health.record_failure(&a);
        assert_eq!(ejections(), 1);

        health.record_success(&a);
        health.record_failure(&a);
        assert_eq!(ejections(), 2, "a quick relapse keeps escalating");

        health.record_success(&a);
        health.state.get_mut(&a).expect("tracked").healthy_since =
            Some(Instant::now() - MAX_EJECTION_BACKOFF);
        health.record_failure(&a);
        assert_eq!(ejections(), 1, "a long stable period starts over");
    }

    #[test]
    fn success_on_stable_address_keeps_none_since() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.record_success(&a);
        let snap = health.snapshot(&a);
        assert_eq!(snap.state, BackendState::Healthy);
        assert!(snap.healthy_since.is_none());
    }

    #[test]
    fn mark_warming_starts_ramp() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a, Duration::from_secs(45));
        let snap = health.snapshot(&a);
        assert_eq!(snap.state, BackendState::Healthy);
        assert!(snap.healthy_since.is_some());
    }

    #[test]
    fn mark_warming_within_window_keeps_ramp_start() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a, Duration::from_secs(45));
        let first = health.snapshot(&a).healthy_since;
        std::thread::sleep(Duration::from_millis(5));
        health.mark_warming(&a, Duration::from_secs(45));
        assert_eq!(
            health.snapshot(&a).healthy_since,
            first,
            "a startup wave must share one ramp origin"
        );
    }

    #[test]
    fn mark_warming_restarts_ramp_on_rewake() {
        let window = Duration::from_millis(10);
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.mark_warming(&a, window);
        let first = health.snapshot(&a).healthy_since.expect("ramping");
        std::thread::sleep(window * 2);
        health.mark_warming(&a, window);
        let second = health.snapshot(&a).healthy_since.expect("ramping");
        assert!(second > first, "re-wake must reposition healthy_since");
    }

    #[test]
    fn prune_sweeps_stable_entries_keeps_meaningful_ones() {
        let health = PassiveBackendHealth::with_threshold(3);
        let stable = addr("stable");
        health.record_failure(&stable);
        health.record_success(&stable);
        let warming = addr("warming");
        health.mark_warming(&warming, Duration::from_secs(45));
        assert_eq!(health.tracked_addresses(), 2);

        let flaky = addr("flaky");
        for _ in 0..=PRUNE_INTERVAL_OPS {
            health.record_failure(&flaky);
        }

        assert_eq!(health.tracked_addresses(), 2, "stable entry must be swept");
        assert_ne!(state(&health, &flaky), BackendState::Healthy);
        assert!(health.snapshot(&warming).healthy_since.is_some());
        assert_eq!(state(&health, &stable), BackendState::Healthy);
    }

    #[test]
    fn drain_survives_traffic_and_is_cleared_only_by_set_drained() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.set_drained(&a, true);
        assert_eq!(state(&health, &a), BackendState::Draining);

        health.record_success(&a);
        assert_eq!(state(&health, &a), BackendState::Draining);
        health.record_failure(&a);
        assert_eq!(state(&health, &a), BackendState::Draining);
        health.mark_warming(&a, Duration::from_secs(45));
        assert_eq!(state(&health, &a), BackendState::Draining);
        health.reset(&a);
        assert_eq!(state(&health, &a), BackendState::Draining);

        health.set_drained(&a, false);
        assert_eq!(state(&health, &a), BackendState::Healthy);
    }

    #[test]
    fn drained_address_survives_prune() {
        let health = PassiveBackendHealth::with_threshold(3);
        let drained = addr("drained");
        health.set_drained(&drained, true);

        let flaky = addr("flaky");
        for _ in 0..=PRUNE_INTERVAL_OPS {
            health.record_failure(&flaky);
        }

        assert_eq!(
            state(&health, &drained),
            BackendState::Draining,
            "a pruned drain would silently rejoin the pool"
        );
    }

    #[test]
    fn reset_zeroes_the_ejection_history() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        health.record_failure(&a);
        health.record_success(&a);
        health.record_failure(&a);
        assert_eq!(health.report(&a).ejections, 2);

        health.reset(&a);
        let report = health.report(&a);
        assert_eq!(report.ejections, 0);
        assert_eq!(report.state, BackendState::Healthy);
        assert!(report.last_failure.is_none());
        assert!(
            report.healthy_since.is_some(),
            "a reinstated address must ramp back up"
        );
    }

    #[test]
    fn drain_outlives_an_address_leaving_every_config() {
        let health = PassiveBackendHealth::new();
        let a = addr("a");
        health.set_drained(&a, true);

        health.retain_known(&HashSet::new());

        assert_eq!(
            state(&health, &a),
            BackendState::Draining,
            "a config that leaves and comes back must not lift an operator drain"
        );
        assert_eq!(health.report(&a).state, BackendState::Draining);
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<BackendState>>);

    impl HealthTransitionListener for Recorder {
        fn on_transition(&self, _address: &ServerAddress, to: BackendState) {
            self.0.lock().unwrap().push(to);
        }
    }

    #[test]
    fn a_drained_address_still_reports_its_health_transitions() {
        let recorder = Arc::new(Recorder::default());
        let health = PassiveBackendHealth::with_threshold(1)
            .with_listener(Arc::clone(&recorder) as Arc<dyn HealthTransitionListener>);
        let a = addr("a");

        health.set_drained(&a, true);
        health.record_failure(&a);
        health.record_success(&a);
        health.set_drained(&a, false);

        assert_eq!(
            *recorder.0.lock().unwrap(),
            [
                BackendState::Draining,
                BackendState::Unhealthy,
                BackendState::Healthy,
                BackendState::Healthy,
            ],
            "drain must not hide ejection and recovery from listeners"
        );
    }

    #[test]
    fn retain_known_drops_removed_addresses() {
        let health = PassiveBackendHealth::with_threshold(1);
        let (kept, gone) = (addr("kept"), addr("gone"));
        health.record_failure(&kept);
        health.record_failure(&gone);
        assert_eq!(health.tracked_addresses(), 2);

        health.retain_known(&HashSet::from([kept.clone()]));
        assert_eq!(health.tracked_addresses(), 1);
        assert_eq!(state(&health, &gone), BackendState::Healthy);
    }

    #[test]
    fn observer_routes_attempts() {
        let health = PassiveBackendHealth::with_threshold(1);
        let a = addr("a");
        let attempt = |error| ConnectAttempt {
            server_id: "test",
            address: &a,
            elapsed: Duration::ZERO,
            error,
        };
        let failed = infrarust_transport::error::TransportError::AllBackendsFailed {
            server_id: "test".to_string(),
        };
        health.on_attempt(&attempt(Some(&failed)));
        assert_eq!(state(&health, &a), BackendState::Unhealthy);
        health.on_attempt(&attempt(None));
        assert_eq!(state(&health, &a), BackendState::Healthy);
    }
}
