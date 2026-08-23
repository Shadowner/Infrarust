//! Level-1 load balancing: address selection among a `ServerConfig`'s
//! backend addresses (replicas of the same logical server).

mod factory;
mod first_available;
mod health;
mod least_conn;
mod load;
mod observer;
mod pending;
mod prober;
mod round_robin;
mod selection;
mod slow_start;

pub use factory::build_load_balancer;
pub use first_available::FirstAvailable;
pub use health::{
    BackendHealthView, BackendReport, BackendState, HealthSnapshot, HealthTransitionListener,
    PassiveBackendHealth,
};
pub use least_conn::LeastConnections;
pub use load::BackendLoad;
#[cfg(feature = "telemetry")]
pub use observer::HealthTransitionMetrics;
pub use observer::{BackendAttemptObserver, HealthTransitionEvents};
pub use pending::{PendingRegistry, PendingTicket, reservation_ttl};
pub use prober::ActiveHealthProber;
pub use round_robin::RoundRobin;
pub use selection::{peek_backend_addresses, select_backend_addresses};
pub use slow_start::SlowStartConfig;
pub(crate) use slow_start::effective_weight;

use std::time::Instant;

use smallvec::SmallVec;

use infrarust_config::ServerAddress;

#[derive(Debug, Clone)]
pub struct BackendCandidate {
    pub address: ServerAddress,
    pub weight: u32,
    pub active_connections: usize,
    pub state: BackendState,
    pub healthy_since: Option<Instant>,
    pub probe: bool,
}

impl BackendCandidate {
    pub const fn is_healthy(&self) -> bool {
        matches!(self.state, BackendState::Healthy)
    }

    pub const fn is_draining(&self) -> bool {
        matches!(self.state, BackendState::Draining)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Advance,
    Peek,
}

/// Ordering strategy for the backend addresses of a `ServerConfig`, applied
/// to each new incoming session.
///
/// Deliberately **synchronous**: `order` is pure CPU work (picking among
/// N addresses) with no I/O — no per-connection `Future` on the hot path.
pub trait LoadBalancer: Send + Sync {
    /// Name for logging and metrics (e.g. "least_conn").
    fn name(&self) -> &'static str;

    /// Orders the selectable candidates, i.e. the healthy ones.
    ///
    /// Strategies only ever see candidates they are allowed to return, so a
    /// drained address cannot leak out of one and the empty case needs no
    /// special handling: with nothing selectable, the order is empty and
    /// [`LoadBalancer::order`] falls back to the last-resort tail on its own.
    ///
    /// Probe hoisting is applied by [`select_backend_addresses`] on top of
    /// this order, so strategies never deal with it either.
    fn order_selectable<'a>(
        &self,
        selectable: &[&'a BackendCandidate],
        mode: SelectionMode,
    ) -> SmallVec<[&'a BackendCandidate; 4]>;

    /// Returns the candidates in the order to try them.
    ///
    /// Contract:
    /// - `candidates` is never empty (guaranteed by the middleware; a
    ///   `ServerConfig` always has ≥ 1 address).
    /// - **Healthy** candidates come first, ordered by the strategy.
    /// - **Ejected** candidates are appended at the tail as a last-resort
    ///   failover, probe-eligible ones first, config order within each group.
    /// - If *all* are ejected: all returned in that tail order. Never an
    ///   empty list — better to try and fail than deny any chance.
    /// - **Draining** candidates are dropped entirely.
    ///   [`select_backend_addresses`] guarantees at least one candidate is
    ///   not draining, so the result still cannot be empty.
    fn order<'a>(
        &self,
        candidates: &'a [BackendCandidate],
        mode: SelectionMode,
    ) -> SmallVec<[&'a BackendCandidate; 4]> {
        let (selectable, last_resort) = split_health(candidates);
        let mut ordered = self.order_selectable(&selectable, mode);
        ordered.extend(last_resort);
        ordered
    }
}

/// Per-address active connection count, fed to [`BackendCandidate`].
///
/// Implemented by the `ConnectionRegistry`; kept as a trait so the
/// selection middleware is testable with a mock.
pub trait AddressConnectionCount: Send + Sync {
    /// Active connections on a precise backend address.
    fn active_connections_for_address(&self, addr: &ServerAddress) -> usize;
}

/// Splits selectable / last-resort, preserving config order within each group.
///
/// The tail keeps probe-eligible addresses ahead of still-ejected ones, so a
/// forced failover prefers the address closest to recovery.
fn split_health(
    candidates: &[BackendCandidate],
) -> (
    SmallVec<[&BackendCandidate; 4]>,
    SmallVec<[&BackendCandidate; 4]>,
) {
    let mut healthy = SmallVec::new();
    let mut probing = SmallVec::new();
    let mut unhealthy: SmallVec<[&BackendCandidate; 4]> = SmallVec::new();
    for c in candidates {
        match c.state {
            BackendState::Healthy => healthy.push(c),
            BackendState::Probing => probing.push(c),
            BackendState::Unhealthy => unhealthy.push(c),
            // Dropped from both groups: a drained address is not even a
            // last-resort failover.
            BackendState::Draining => {}
        }
    }
    probing.extend(unhealthy);
    (healthy, probing)
}

#[cfg(test)]
pub(crate) fn test_candidate(host: &str, healthy: bool) -> BackendCandidate {
    test_candidate_in(
        host,
        if healthy {
            BackendState::Healthy
        } else {
            BackendState::Unhealthy
        },
    )
}

#[cfg(test)]
pub(crate) fn test_candidate_in(host: &str, state: BackendState) -> BackendCandidate {
    BackendCandidate {
        address: ServerAddress {
            host: host.to_string(),
            port: 25565,
        },
        weight: 1,
        active_connections: 0,
        state,
        healthy_since: None,
        probe: false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn strategies() -> Vec<Box<dyn LoadBalancer>> {
        vec![
            Box::new(FirstAvailable),
            Box::new(RoundRobin::new(None)),
            Box::new(LeastConnections::new(None)),
        ]
    }

    #[test]
    fn test_order_never_empty() {
        let candidates = vec![test_candidate("a", true), test_candidate("b", false)];
        for lb in strategies() {
            assert!(
                !lb.order(&candidates, SelectionMode::Advance).is_empty(),
                "{}",
                lb.name()
            );
        }
    }

    #[test]
    fn test_order_healthy_before_unhealthy() {
        let candidates = vec![
            test_candidate("a", false),
            test_candidate("b", true),
            test_candidate("c", false),
            test_candidate("d", true),
        ];
        for lb in strategies() {
            let ordered = lb.order(&candidates, SelectionMode::Advance);
            assert_eq!(ordered.len(), 4, "{}", lb.name());
            assert!(
                ordered[0].is_healthy() && ordered[1].is_healthy(),
                "{}",
                lb.name()
            );
            assert!(
                !ordered[2].is_healthy() && !ordered[3].is_healthy(),
                "{}",
                lb.name()
            );
        }
    }

    #[test]
    fn test_order_all_unhealthy_returns_all() {
        let candidates = vec![
            test_candidate("a", false),
            test_candidate("b", false),
            test_candidate("c", false),
        ];
        for lb in strategies() {
            let ordered = lb.order(&candidates, SelectionMode::Advance);
            let hosts: Vec<&str> = ordered.iter().map(|c| c.address.host.as_str()).collect();
            assert_eq!(hosts, ["a", "b", "c"], "{}", lb.name());
        }
    }

    #[test]
    fn test_order_puts_probing_ahead_of_unhealthy() {
        let candidates = vec![
            test_candidate_in("a", BackendState::Unhealthy),
            test_candidate_in("b", BackendState::Probing),
            test_candidate_in("c", BackendState::Healthy),
        ];
        for lb in strategies() {
            let ordered = lb.order(&candidates, SelectionMode::Advance);
            let hosts: Vec<&str> = ordered.iter().map(|c| c.address.host.as_str()).collect();
            assert_eq!(hosts, ["c", "b", "a"], "{}", lb.name());
        }
    }

    #[test]
    fn test_split_health_preserves_config_order() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate("b", false),
            test_candidate("c", true),
            test_candidate_in("d", BackendState::Probing),
        ];
        let (healthy, tail) = split_health(&candidates);
        let h: Vec<&str> = healthy.iter().map(|c| c.address.host.as_str()).collect();
        let t: Vec<&str> = tail.iter().map(|c| c.address.host.as_str()).collect();
        assert_eq!(h, ["a", "c"]);
        assert_eq!(t, ["d", "b"], "probe-eligible before still-ejected");
    }

    #[test]
    fn test_split_health_drops_draining_from_both_groups() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate_in("b", BackendState::Draining),
            test_candidate("c", false),
        ];
        let (healthy, tail) = split_health(&candidates);
        let h: Vec<&str> = healthy.iter().map(|c| c.address.host.as_str()).collect();
        let t: Vec<&str> = tail.iter().map(|c| c.address.host.as_str()).collect();
        assert_eq!(h, ["a"]);
        assert_eq!(t, ["c"], "a drained address is not a last-resort failover");
    }

    #[test]
    fn test_order_drops_draining_with_nothing_healthy_left() {
        let candidates = vec![
            test_candidate_in("drained", BackendState::Draining),
            test_candidate_in("ejected", BackendState::Unhealthy),
            test_candidate_in("probing", BackendState::Probing),
        ];
        for lb in strategies() {
            let hosts: Vec<&str> = lb
                .order(&candidates, SelectionMode::Advance)
                .iter()
                .map(|c| c.address.host.as_str())
                .collect();
            assert_eq!(
                hosts,
                ["probing", "ejected"],
                "{}: a drained address is not a last-resort failover",
                lb.name()
            );
        }
    }

    #[test]
    fn test_order_of_only_draining_candidates_is_empty() {
        let candidates = vec![
            test_candidate_in("a", BackendState::Draining),
            test_candidate_in("b", BackendState::Draining),
        ];
        for lb in strategies() {
            assert!(
                lb.order(&candidates, SelectionMode::Advance).is_empty(),
                "{}",
                lb.name()
            );
        }
    }

    #[test]
    fn test_order_never_returns_a_draining_candidate() {
        let candidates = vec![
            test_candidate_in("a", BackendState::Draining),
            test_candidate("b", true),
        ];
        for lb in strategies() {
            let hosts: Vec<&str> = lb
                .order(&candidates, SelectionMode::Advance)
                .iter()
                .map(|c| c.address.host.as_str())
                .collect();
            assert_eq!(hosts, ["b"], "{}", lb.name());
        }
    }
}
