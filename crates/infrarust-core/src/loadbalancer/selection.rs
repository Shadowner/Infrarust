//! Shared backend selection: candidate snapshot + strategy ordering.
use smallvec::SmallVec;

use infrarust_config::{ServerAddress, ServerConfig};

use super::{
    AddressConnectionCount, BackendCandidate, BackendHealthView, BackendState, LoadBalancer,
};

pub fn select_backend_addresses(
    server: &ServerConfig,
    load_balancer: &dyn LoadBalancer,
    counts: &dyn AddressConnectionCount,
    health: &dyn BackendHealthView,
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
                probe: snapshot.state == BackendState::Probing && health.claim_probe(&wa.address),
            }
        })
        .collect();

    if !candidates.iter().any(BackendCandidate::is_healthy) {
        for c in &mut candidates {
            c.state = BackendState::Healthy;
        }
    }

    let ordered = load_balancer.order(&candidates);

    ordered
        .iter()
        .filter(|c| c.probe)
        .chain(ordered.iter().filter(|c| !c.probe))
        .map(|c| c.address.clone())
        .collect()
}
