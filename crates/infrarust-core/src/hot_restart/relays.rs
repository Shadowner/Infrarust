use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use infrarust_transport::Freeze;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::package::FrozenSession;
use crate::player::ShutdownKind;

#[derive(Debug)]
pub struct RelayRegistry {
    sessions: CancellationToken,
    pipeline: CancellationToken,
    logins: Mutex<HashMap<u64, CancellationToken>>,
    next_login: AtomicU64,
    in_flight: AtomicUsize,
    settled: Notify,
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
        let sessions = CancellationToken::new();
        let pipeline = sessions.child_token();
        Self {
            sessions,
            pipeline,
            logins: Mutex::new(HashMap::new()),
            next_login: AtomicU64::new(0),
            in_flight: AtomicUsize::new(0),
            settled: Notify::new(),
            freeze: CancellationToken::new(),
            write_deadline,
            active: AtomicUsize::new(0),
            drained: Notify::new(),
            frozen: Mutex::new(Vec::new()),
            restarting: AtomicBool::new(false),
        }
    }

    pub const fn sessions(&self) -> &CancellationToken {
        &self.sessions
    }

    pub const fn pipeline(&self) -> &CancellationToken {
        &self.pipeline
    }

    pub fn close_pipeline(&self) {
        self.pipeline.cancel();
        let cut = std::mem::take(&mut *self.logins());
        for login in cut.into_values() {
            login.cancel();
        }
    }

    pub fn link_login(self: &Arc<Self>) -> (CancellationToken, LoginLink) {
        let token = self.sessions.child_token();
        let key = self.next_login.fetch_add(1, Ordering::Relaxed);
        self.logins().insert(key, token.clone());
        if self.pipeline.is_cancelled() && self.logins().remove(&key).is_some() {
            token.cancel();
        }
        let link = LoginLink {
            registry: Arc::clone(self),
            key: Some(key),
        };
        (token, link)
    }

    pub fn linked_logins(&self) -> usize {
        self.logins().len()
    }

    fn logins(&self) -> MutexGuard<'_, HashMap<u64, CancellationToken>> {
        self.logins.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn in_flight_guard(self: &Arc<Self>) -> InFlight {
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        InFlight {
            registry: Arc::clone(self),
        }
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    pub async fn settled(&self, deadline: tokio::time::Instant) -> usize {
        wait_for_zero(&self.in_flight, &self.settled, deadline).await
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
        wait_for_zero(&self.active, &self.drained, deadline).await
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

async fn wait_for_zero(
    counter: &AtomicUsize,
    zero: &Notify,
    deadline: tokio::time::Instant,
) -> usize {
    loop {
        let notified = zero.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if counter.load(Ordering::Acquire) == 0 {
            return 0;
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            return counter.load(Ordering::Acquire);
        }
    }
}

#[derive(Debug)]
#[must_use = "dropping the link detaches the login from the pipeline"]
pub struct LoginLink {
    registry: Arc<RelayRegistry>,
    key: Option<u64>,
}

impl LoginLink {
    pub fn detach(mut self) -> bool {
        self.unlink()
    }

    fn unlink(&mut self) -> bool {
        self.key
            .take()
            .is_some_and(|key| self.registry.logins().remove(&key).is_some())
    }
}

impl Drop for LoginLink {
    fn drop(&mut self) {
        self.unlink();
    }
}

#[derive(Debug)]
#[must_use = "dropping the guard settles the connection"]
pub struct InFlight {
    registry: Arc<RelayRegistry>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if self.registry.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.registry.settled.notify_waiters();
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
    fn closing_the_pipeline_cuts_linked_logins_but_spares_detached_ones() {
        let registry = Arc::new(RelayRegistry::default());
        let (forwarding, link) = registry.link_login();
        let (logging_in, _held) = registry.link_login();
        let connection = registry.pipeline().child_token();
        assert!(link.detach());
        assert_eq!(registry.linked_logins(), 1);

        registry.close_pipeline();

        assert!(connection.is_cancelled());
        assert!(logging_in.is_cancelled());
        assert!(!forwarding.is_cancelled());
        assert!(!registry.sessions().is_cancelled());
        assert_eq!(registry.linked_logins(), 0);
        registry.sessions().cancel();
        assert!(forwarding.is_cancelled());
    }

    #[test]
    fn a_login_linked_after_the_pipeline_closed_starts_cut() {
        let registry = Arc::new(RelayRegistry::default());
        registry.close_pipeline();

        let (token, link) = registry.link_login();

        assert!(token.is_cancelled());
        assert!(!link.detach());
        assert_eq!(registry.linked_logins(), 0);
    }

    #[test]
    fn a_cut_login_reports_it_on_detach() {
        let registry = Arc::new(RelayRegistry::default());
        let (token, link) = registry.link_login();
        registry.close_pipeline();
        assert!(token.is_cancelled());
        assert!(!link.detach());
    }

    #[test]
    fn dropping_a_link_unlinks_the_login() {
        let registry = Arc::new(RelayRegistry::default());
        let (token, link) = registry.link_login();
        drop(link);
        assert_eq!(registry.linked_logins(), 0);
        registry.close_pipeline();
        assert!(!token.is_cancelled());
    }

    #[tokio::test]
    async fn settled_waits_for_every_connection_in_flight() {
        let registry = Arc::new(RelayRegistry::default());
        let first = registry.in_flight_guard();
        let second = registry.in_flight_guard();
        assert_eq!(registry.in_flight(), 2);

        let waiter = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .settled(tokio::time::Instant::now() + Duration::from_secs(5))
                    .await
            })
        };
        drop(first);
        tokio::task::yield_now().await;
        drop(second);
        assert_eq!(waiter.await.ok(), Some(0));
        assert_eq!(registry.in_flight(), 0);

        let _late = registry.in_flight_guard();
        let left = registry
            .settled(tokio::time::Instant::now() + Duration::from_millis(20))
            .await;
        assert_eq!(left, 1);
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
