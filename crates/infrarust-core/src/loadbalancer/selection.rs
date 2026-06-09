//! Shared backend selection: candidate snapshot + strategy ordering.
use smallvec::SmallVec;

use infrarust_config::{ServerAddress, ServerConfig};

use super::{AddressConnectionCount, BackendCandidate, BackendHealthView, LoadBalancer};

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
    let candidates: Vec<BackendCandidate> = server
        .addresses
        .iter()
        .map(|wa| {
            let (healthy, healthy_since) = health.snapshot(&wa.address);
            BackendCandidate {
                address: wa.address.clone(),
                weight: wa.weight,
                active_connections: counts.active_connections_for_address(&wa.address),
                healthy,
                healthy_since,
            }
        })
        .collect();

    load_balancer
        .order(&candidates)
        .iter()
        .map(|c| c.address.clone())
        .collect()
}
