//! In-flight logins, so concurrent selections do not pile onto one address.
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use infrarust_config::ServerAddress;

use super::{AddressConnectionCount, BackendLoad};

const SWEEP_INTERVAL_OPS: u32 = 64;

struct Reservation {
    address: ServerAddress,
    expires_at: Instant,
}

/// Load as the balancer should see it: sessions already attached to an
/// address, plus logins currently negotiating their way to one.
pub struct PendingRegistry {
    attached: Arc<dyn AddressConnectionCount>,
    pending: BackendLoad,
    reservations: DashMap<u64, Reservation>,
    next_id: AtomicU64,
    ops_since_sweep: AtomicU32,
    ttl: Duration,
}

impl PendingRegistry {
    pub fn new(attached: Arc<dyn AddressConnectionCount>, ttl: Duration) -> Self {
        Self {
            attached,
            pending: BackendLoad::new(),
            reservations: DashMap::new(),
            next_id: AtomicU64::new(0),
            ops_since_sweep: AtomicU32::new(0),
            ttl,
        }
    }

    pub fn reserve(self: &Arc<Self>, address: &ServerAddress) -> PendingTicket {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending.acquire(address);
        self.reservations.insert(
            id,
            Reservation {
                address: address.clone(),
                expires_at: Instant::now() + self.ttl,
            },
        );
        self.maybe_sweep();
        PendingTicket {
            registry: Arc::clone(self),
            id,
        }
    }

    /// Idempotent: the reservation entry is the single source of truth, so an
    /// expiry sweep and a ticket drop can never release the same slot twice.
    fn release(&self, id: u64) {
        if let Some((_, reservation)) = self.reservations.remove(&id) {
            self.pending.release(&reservation.address);
        }
    }

    fn maybe_sweep(&self) {
        if self.ops_since_sweep.fetch_add(1, Ordering::Relaxed) < SWEEP_INTERVAL_OPS {
            return;
        }
        self.ops_since_sweep.store(0, Ordering::Relaxed);
        let now = Instant::now();
        let expired: Vec<u64> = self
            .reservations
            .iter()
            .filter(|r| r.expires_at <= now)
            .map(|r| *r.key())
            .collect();
        for id in expired {
            self.release(id);
        }
    }

    #[cfg(test)]
    fn pending_for(&self, address: &ServerAddress) -> usize {
        self.pending.active_connections_for_address(address)
    }
}

impl AddressConnectionCount for PendingRegistry {
    fn active_connections_for_address(&self, addr: &ServerAddress) -> usize {
        self.attached.active_connections_for_address(addr)
            + self.pending.active_connections_for_address(addr)
    }
}

/// Releases its reservation when dropped. Only an optimization over the TTL.
#[must_use = "dropping the ticket releases the reservation"]
pub struct PendingTicket {
    registry: Arc<PendingRegistry>,
    id: u64,
}

impl Drop for PendingTicket {
    fn drop(&mut self) {
        self.registry.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    fn registry(ttl: Duration) -> Arc<PendingRegistry> {
        Arc::new(PendingRegistry::new(Arc::new(BackendLoad::new()) as _, ttl))
    }

    #[test]
    fn reservation_counts_until_the_ticket_drops() {
        let registry = registry(Duration::from_secs(10));
        let a = addr("a");
        let ticket = registry.reserve(&a);
        assert_eq!(registry.active_connections_for_address(&a), 1);
        drop(ticket);
        assert_eq!(registry.active_connections_for_address(&a), 0);
    }

    #[test]
    fn counts_add_up_with_attached_sessions() {
        let attached = Arc::new(BackendLoad::new());
        let registry = Arc::new(PendingRegistry::new(
            Arc::clone(&attached) as _,
            Duration::from_secs(10),
        ));
        let a = addr("a");
        attached.acquire(&a);
        let _ticket = registry.reserve(&a);
        assert_eq!(registry.active_connections_for_address(&a), 2);
    }

    #[test]
    fn expired_reservations_are_swept_without_their_ticket() {
        let registry = registry(Duration::ZERO);
        let a = addr("a");
        let leaked: Vec<PendingTicket> = (0..=SWEEP_INTERVAL_OPS)
            .map(|_| registry.reserve(&a))
            .collect();
        assert_eq!(
            registry.pending_for(&a),
            0,
            "expired reservations must not survive their TTL"
        );
        drop(leaked);
        assert_eq!(registry.pending_for(&a), 0, "double release must be inert");
    }

    #[test]
    fn ticket_drop_after_sweep_is_inert() {
        let registry = registry(Duration::ZERO);
        let a = addr("a");
        let ticket = registry.reserve(&a);
        for _ in 0..=SWEEP_INTERVAL_OPS {
            drop(registry.reserve(&addr("other")));
        }
        drop(ticket);
        assert_eq!(registry.pending_for(&a), 0);
        assert_eq!(registry.pending_for(&addr("other")), 0);
    }
}
