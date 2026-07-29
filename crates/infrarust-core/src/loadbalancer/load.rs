//! Live session count per backend address.

use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

use infrarust_config::ServerAddress;

use super::AddressConnectionCount;

#[derive(Debug, Default)]
pub struct BackendLoad {
    counts: DashMap<ServerAddress, AtomicUsize>,
}

impl BackendLoad {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn acquire(&self, addr: &ServerAddress) {
        if let Some(count) = self.counts.get(addr) {
            count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counts
                .entry(addr.clone())
                .or_default()
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn release(&self, addr: &ServerAddress) {
        self.counts
            .remove_if(addr, |_, count| count.fetch_sub(1, Ordering::Relaxed) == 1);
    }

    #[cfg(test)]
    fn tracked_addresses(&self) -> usize {
        self.counts.len()
    }
}

impl AddressConnectionCount for BackendLoad {
    fn active_connections_for_address(&self, addr: &ServerAddress) -> usize {
        self.counts
            .get(addr)
            .map_or(0, |count| count.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn addr(host: &str) -> ServerAddress {
        ServerAddress {
            host: host.to_string(),
            port: 25565,
        }
    }

    #[test]
    fn acquire_and_release_round_trip() {
        let load = BackendLoad::new();
        let a = addr("a");
        assert_eq!(load.active_connections_for_address(&a), 0);
        load.acquire(&a);
        load.acquire(&a);
        assert_eq!(load.active_connections_for_address(&a), 2);
        load.release(&a);
        assert_eq!(load.active_connections_for_address(&a), 1);
        load.release(&a);
        assert_eq!(load.active_connections_for_address(&a), 0);
        assert_eq!(load.tracked_addresses(), 0, "empty entry must be dropped");
    }

    #[test]
    fn release_of_untracked_address_is_inert() {
        let load = BackendLoad::new();
        load.release(&addr("ghost"));
        assert_eq!(load.active_connections_for_address(&addr("ghost")), 0);
    }

    #[test]
    fn concurrent_acquire_release_returns_to_zero() {
        let load = Arc::new(BackendLoad::new());
        let a = addr("a");
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let load = Arc::clone(&load);
                let a = a.clone();
                thread::spawn(move || {
                    for _ in 0..1000 {
                        load.acquire(&a);
                        load.release(&a);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked");
        }
        assert_eq!(load.active_connections_for_address(&a), 0);
        assert_eq!(load.tracked_addresses(), 0);
    }
}
