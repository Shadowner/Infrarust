//! Level-1 load balancing: address selection among a `ServerConfig`'s
//! backend addresses (replicas of the same logical server).

mod factory;
mod first_available;
mod health;
mod least_conn;
mod round_robin;
mod slow_start;

pub use factory::build_load_balancer;
pub use first_available::FirstAvailable;
pub use health::{BackendHealthView, PassiveBackendHealth};
pub use least_conn::LeastConnections;
pub use round_robin::RoundRobin;
pub use slow_start::SlowStartConfig;

use std::time::Instant;

use smallvec::SmallVec;

use infrarust_config::ServerAddress;

#[derive(Debug, Clone)]
pub struct BackendCandidate {
    pub address: ServerAddress,
    pub weight: u32,
    pub active_connections: usize,
    pub healthy: bool,
    pub healthy_since: Option<Instant>,
}

/// Ordering strategy for the backend addresses of a `ServerConfig`, applied
/// to each new incoming session.
///
/// Deliberately **synchronous**: `order` is pure CPU work (picking among
/// N addresses) with no I/O — no per-connection `Future` on the hot path.
pub trait LoadBalancer: Send + Sync {
    /// Name for logging and metrics (e.g. "least_conn").
    fn name(&self) -> &'static str;

    /// Returns the candidates in the order to try them.
    ///
    /// Contract:
    /// - `candidates` is never empty (guaranteed by the middleware; a
    ///   `ServerConfig` always has ≥ 1 address).
    /// - **Healthy** candidates come first, ordered by the strategy.
    /// - **Unhealthy** candidates are appended at the tail (config order)
    ///   as a last-resort failover.
    /// - If *all* are unhealthy: all returned in config order. Never an
    ///   empty list — better to try and fail than deny any chance.
    fn order<'a>(&self, candidates: &'a [BackendCandidate]) -> SmallVec<[&'a BackendCandidate; 4]>;
}

/// Per-address active connection count, fed to [`BackendCandidate`].
///
/// Implemented by the `ConnectionRegistry`; kept as a trait so the
/// selection middleware is testable with a mock.
pub trait AddressConnectionCount: Send + Sync {
    /// Active connections on a precise backend address.
    fn active_connections_for_address(&self, addr: &ServerAddress) -> usize;
}

/// Splits healthy / unhealthy, preserving config order within each group.
fn split_health(
    candidates: &[BackendCandidate],
) -> (
    SmallVec<[&BackendCandidate; 4]>,
    SmallVec<[&BackendCandidate; 4]>,
) {
    let mut healthy = SmallVec::new();
    let mut unhealthy = SmallVec::new();
    for c in candidates {
        if c.healthy {
            healthy.push(c);
        } else {
            unhealthy.push(c);
        }
    }
    (healthy, unhealthy)
}

#[cfg(test)]
pub(crate) fn test_candidate(host: &str, healthy: bool) -> BackendCandidate {
    BackendCandidate {
        address: ServerAddress {
            host: host.to_string(),
            port: 25565,
        },
        weight: 1,
        active_connections: 0,
        healthy,
        healthy_since: None,
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
            assert!(!lb.order(&candidates).is_empty(), "{}", lb.name());
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
            let ordered = lb.order(&candidates);
            assert_eq!(ordered.len(), 4, "{}", lb.name());
            assert!(ordered[0].healthy && ordered[1].healthy, "{}", lb.name());
            assert!(!ordered[2].healthy && !ordered[3].healthy, "{}", lb.name());
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
            let ordered = lb.order(&candidates);
            let hosts: Vec<&str> = ordered.iter().map(|c| c.address.host.as_str()).collect();
            assert_eq!(hosts, ["a", "b", "c"], "{}", lb.name());
        }
    }

    #[test]
    fn test_split_health_preserves_config_order() {
        let candidates = vec![
            test_candidate("a", true),
            test_candidate("b", false),
            test_candidate("c", true),
            test_candidate("d", false),
        ];
        let (healthy, unhealthy) = split_health(&candidates);
        let h: Vec<&str> = healthy.iter().map(|c| c.address.host.as_str()).collect();
        let u: Vec<&str> = unhealthy.iter().map(|c| c.address.host.as_str()).collect();
        assert_eq!(h, ["a", "c"]);
        assert_eq!(u, ["b", "d"]);
    }
}
