//! Shared backend selection: candidate snapshot + strategy ordering.
use smallvec::SmallVec;

use infrarust_config::{ServerAddress, ServerConfig};

use super::{
    AddressConnectionCount, BackendCandidate, BackendHealthView, BackendState, LoadBalancer,
    SelectionMode,
};

pub fn select_backend_addresses(
    server: &ServerConfig,
    load_balancer: &dyn LoadBalancer,
    counts: &dyn AddressConnectionCount,
    health: &dyn BackendHealthView,
) -> SmallVec<[ServerAddress; 4]> {
    arrange(
        server,
        load_balancer,
        counts,
        health,
        SelectionMode::Advance,
    )
}

pub fn peek_backend_addresses(
    server: &ServerConfig,
    load_balancer: &dyn LoadBalancer,
    counts: &dyn AddressConnectionCount,
    health: &dyn BackendHealthView,
) -> SmallVec<[ServerAddress; 4]> {
    arrange(server, load_balancer, counts, health, SelectionMode::Peek)
}

fn arrange(
    server: &ServerConfig,
    load_balancer: &dyn LoadBalancer,
    counts: &dyn AddressConnectionCount,
    health: &dyn BackendHealthView,
    mode: SelectionMode,
) -> SmallVec<[ServerAddress; 4]> {
    // Shortcut: a single address -> nothing to balance.
    if let [only] = server.addresses.as_slice() {
        return smallvec::smallvec![only.address.clone()];
    }

    // Snapshot: config (weights) x connection counts x health view.
    let mut candidates: Vec<BackendCandidate> = server
        .addresses
        .iter()
        .map(|wa| {
            let snapshot = health.snapshot(&wa.address);
            BackendCandidate {
                address: wa.address.clone(),
                weight: wa.weight,
                active_connections: counts.active_connections_for_address(&wa.address),
                state: snapshot.state,
                healthy_since: snapshot.healthy_since,
                probe: snapshot.state == BackendState::Probing,
            }
        })
        .collect();

    // Nothing selectable left: reinstate the whole pool rather than deny the
    // connection. Drained addresses are spared unless draining is all there
    // is, so a drain can never black-hole a server.
    if !candidates.iter().any(BackendCandidate::is_healthy) {
        let spare_drained = candidates.iter().any(|c| !c.is_draining());
        for c in &mut candidates {
            if !(spare_drained && c.is_draining()) {
                c.state = BackendState::Healthy;
            }
        }
    }

    let ordered = load_balancer.order(&candidates, mode);

    let hoisted = match mode {
        SelectionMode::Advance => ordered
            .iter()
            .position(|c| c.probe && health.claim_probe(&c.address)),
        SelectionMode::Peek => None,
    };

    let Some(hoisted) = hoisted else {
        return ordered.iter().map(|c| c.address.clone()).collect();
    };
    std::iter::once(ordered[hoisted])
        .chain(
            ordered
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != hoisted)
                .map(|(_, c)| *c),
        )
        .map(|c| c.address.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::collections::HashMap;

    use super::*;
    use crate::loadbalancer::{FirstAvailable, HealthSnapshot, RoundRobin};

    struct NoConnections;

    impl AddressConnectionCount for NoConnections {
        fn active_connections_for_address(&self, _addr: &ServerAddress) -> usize {
            0
        }
    }

    struct FixedHealth(HashMap<ServerAddress, BackendState>);

    impl BackendHealthView for FixedHealth {
        fn snapshot(&self, addr: &ServerAddress) -> HealthSnapshot {
            HealthSnapshot {
                state: self.0.get(addr).copied().unwrap_or(BackendState::Healthy),
                healthy_since: None,
            }
        }
    }

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    fn server() -> ServerConfig {
        toml::from_str(
            "name = \"lobby\"\ndomains = [\"lobby.test\"]\naddresses = [\"a:25565\", \"b:25565\"]\n",
        )
        .unwrap()
    }

    fn selected(states: &[(&str, BackendState)]) -> Vec<String> {
        let health = FixedHealth(
            states
                .iter()
                .map(|(host, state)| (addr(host), *state))
                .collect(),
        );
        select_backend_addresses(&server(), &FirstAvailable, &NoConnections, &health)
            .into_iter()
            .map(|a| a.host)
            .collect()
    }

    #[test]
    fn status_pings_never_advance_the_login_rotation() {
        let health = FixedHealth(HashMap::new());
        let server = server();
        let lb = RoundRobin::new(None);

        let mut logins = Vec::new();
        for round in 0..6 {
            for _ in 0..round {
                peek_backend_addresses(&server, &lb, &NoConnections, &health);
            }
            logins.push(
                select_backend_addresses(&server, &lb, &NoConnections, &health)[0]
                    .host
                    .clone(),
            );
        }

        assert_ne!(logins[0], logins[1]);
        let expected: Vec<String> = (0..6).map(|i| logins[i % 2].clone()).collect();
        assert_eq!(
            logins, expected,
            "ping traffic must not steer the login rotation"
        );
    }

    #[test]
    fn draining_leaves_the_selectable_pool() {
        assert_eq!(selected(&[("a", BackendState::Draining)]), ["b"]);
    }

    #[test]
    fn draining_is_not_a_last_resort_failover() {
        assert_eq!(
            selected(&[
                ("a", BackendState::Draining),
                ("b", BackendState::Unhealthy)
            ]),
            ["b"],
            "the ejected address is preferred over the drained one"
        );
    }

    #[test]
    fn draining_every_address_does_not_black_hole_the_server() {
        assert_eq!(
            selected(&[("a", BackendState::Draining), ("b", BackendState::Draining)]),
            ["a", "b"],
            "with nothing else left, drain is ignored"
        );
    }
}
