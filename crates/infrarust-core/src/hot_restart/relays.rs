use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use infrarust_transport::Freeze;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::package::FrozenSession;
use crate::player::ShutdownKind;

#[derive(Debug)]
pub struct RelayRegistry {
    freeze: CancellationToken,
    write_deadline: Duration,
    active: AtomicUsize,
    drained: Notify,
    frozen: Mutex<Vec<FrozenSession>>,
    restarting: AtomicBool,
}

impl Default for RelayRegistry {
    fn default() -> Self {
        Self::new(Freeze::DEFAULT_WRITE_DEADLINE)
    }
}

impl RelayRegistry {
    pub fn new(write_deadline: Duration) -> Self {
        Self {
            freeze: CancellationToken::new(),
            write_deadline,
            active: AtomicUsize::new(0),
            drained: Notify::new(),
            frozen: Mutex::new(Vec::new()),
            restarting: AtomicBool::new(false),
        }
    }

    pub fn freeze(&self) -> Freeze {
        Freeze::new(self.freeze.clone(), self.write_deadline)
    }

    pub fn ticket(self: &Arc<Self>) -> RelayTicket {
        self.active.fetch_add(1, Ordering::AcqRel);
        RelayTicket {
            registry: Arc::clone(self),
        }
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub fn begin_freeze(&self) {
        self.freeze.cancel();
    }

    pub fn is_freezing(&self) -> bool {
        self.freeze.is_cancelled()
    }

    pub async fn drained(&self, deadline: tokio::time::Instant) -> usize {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let remaining = self.active();
            if remaining == 0 {
                return 0;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return self.active();
            }
        }
    }

    pub fn push_frozen(&self, session: FrozenSession) {
        self.frozen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session);
    }

    pub fn take_frozen(&self) -> Vec<FrozenSession> {
        std::mem::take(&mut *self.frozen.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn set_notice(&self, kind: ShutdownKind) {
        self.restarting
            .store(kind == ShutdownKind::Restart, Ordering::Release);
    }

    pub fn notice(&self) -> ShutdownKind {
        if self.restarting.load(Ordering::Acquire) {
            ShutdownKind::Restart
        } else {
            ShutdownKind::Stop
        }
    }
}

#[derive(Debug)]
pub struct RelayTicket {
    registry: Arc<RelayRegistry>,
}

impl Drop for RelayTicket {
    fn drop(&mut self) {
        if self.registry.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.registry.drained.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drained_returns_once_every_ticket_is_gone() {
        let registry = Arc::new(RelayRegistry::default());
        let first = registry.ticket();
        let second = registry.ticket();
        assert_eq!(registry.active(), 2);

        let waiter = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .drained(tokio::time::Instant::now() + Duration::from_secs(5))
                    .await
            })
        };
        drop(first);
        tokio::task::yield_now().await;
        drop(second);
        assert_eq!(waiter.await.ok(), Some(0));
    }

    #[tokio::test]
    async fn drained_reports_the_stragglers_at_the_deadline() {
        let registry = Arc::new(RelayRegistry::default());
        let _held = registry.ticket();
        let remaining = registry
            .drained(tokio::time::Instant::now() + Duration::from_millis(20))
            .await;
        assert_eq!(remaining, 1);
    }

    #[test]
    fn freezing_cancels_every_handed_out_freeze() {
        let registry = RelayRegistry::default();
        let freeze = registry.freeze();
        assert!(!freeze.token().is_cancelled());
        assert!(!registry.is_freezing());
        registry.begin_freeze();
        assert!(freeze.token().is_cancelled());
        assert!(registry.is_freezing());
    }

    #[test]
    fn the_notice_follows_the_shutdown_kind() {
        let registry = RelayRegistry::default();
        assert_eq!(registry.notice(), ShutdownKind::Stop);
        registry.set_notice(ShutdownKind::Restart);
        assert_eq!(registry.notice(), ShutdownKind::Restart);
        assert_eq!(registry.notice().reason(), crate::player::RESTART_REASON);
    }
}
