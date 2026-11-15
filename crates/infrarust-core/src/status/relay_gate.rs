//! Admits one status relay at a time per server and remembers recent failures,
//! so that pings against a silent backend neither queue up nor reconnect.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::time::Instant;

const UNREACHABLE_TTL: Duration = Duration::from_secs(3);
const MAX_WAIT: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub struct RelayGate {
    slots: DashMap<String, Arc<Mutex<Option<Instant>>>>,
}

pub enum RelayTurn {
    Leader(RelayLeader),
    Unreachable,
    Busy,
}

/// Holds the relay slot of a server. Dropping it leaves the previous verdict in place.
pub struct RelayLeader(OwnedMutexGuard<Option<Instant>>);

impl RelayLeader {
    pub fn succeeded(mut self) {
        *self.0 = None;
    }

    pub fn failed(mut self) {
        *self.0 = Some(Instant::now() + UNREACHABLE_TTL);
    }
}

impl RelayGate {
    pub async fn enter(&self, server: &str) -> RelayTurn {
        let slot = Arc::clone(&self.slots.entry(server.to_string()).or_default());
        let Ok(guard) = tokio::time::timeout(MAX_WAIT, slot.lock_owned()).await else {
            return RelayTurn::Busy;
        };
        match *guard {
            Some(until) if Instant::now() < until => RelayTurn::Unreachable,
            _ => RelayTurn::Leader(RelayLeader(guard)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    async fn leader(gate: &RelayGate, server: &str) -> RelayLeader {
        match gate.enter(server).await {
            RelayTurn::Leader(leader) => leader,
            RelayTurn::Unreachable => panic!("{server} is marked unreachable"),
            RelayTurn::Busy => panic!("{server} is busy"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_gives_up_while_the_leader_is_still_relaying() {
        let gate = RelayGate::default();
        let _leader = leader(&gate, "lobby").await;

        let started = Instant::now();
        assert!(matches!(gate.enter("lobby").await, RelayTurn::Busy));
        assert_eq!(started.elapsed(), MAX_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_is_remembered_until_its_ttl_ends() {
        let gate = RelayGate::default();
        leader(&gate, "lobby").await.failed();

        assert!(matches!(gate.enter("lobby").await, RelayTurn::Unreachable));
        tokio::time::advance(UNREACHABLE_TTL).await;
        assert!(matches!(gate.enter("lobby").await, RelayTurn::Leader(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_success_clears_the_failure_and_servers_do_not_share_a_slot() {
        let gate = RelayGate::default();
        leader(&gate, "lobby").await.failed();
        let _other = leader(&gate, "other").await;

        tokio::time::advance(UNREACHABLE_TTL).await;
        leader(&gate, "lobby").await.succeeded();
        assert!(matches!(gate.enter("lobby").await, RelayTurn::Leader(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiter_learns_the_failure_of_the_leader_it_waited_for() {
        let gate = Arc::new(RelayGate::default());
        let running = leader(&gate, "lobby").await;
        let waiting = Arc::clone(&gate);
        let waiter = tokio::spawn(async move { waiting.enter("lobby").await });
        tokio::task::yield_now().await;

        running.failed();
        assert!(matches!(waiter.await.unwrap(), RelayTurn::Unreachable));
    }
}
