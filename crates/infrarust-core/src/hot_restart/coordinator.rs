use std::fmt;
use std::sync::Arc;

use infrarust_api::services::proxy_control::{ProxyRestart, RestartRequestError};
use tokio::sync::watch;

use super::platform::RestartPlan;
use super::preflight::Preflight;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RestartPhase {
    Idle,
    Preflight,
    Draining,
    Closing,
    Freezing,
    HandingOver,
}

#[derive(Debug, Clone)]
struct HubState {
    phase: RestartPhase,
    pending: bool,
    stopping: bool,
    last_failure: Option<String>,
}

pub struct RestartHub {
    plan: RestartPlan,
    preflight: Arc<dyn Preflight>,
    state: watch::Sender<HubState>,
}

impl fmt::Debug for RestartHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        f.debug_struct("RestartHub")
            .field("target", &self.plan.target)
            .field("phase", &state.phase)
            .field("pending", &state.pending)
            .field("stopping", &state.stopping)
            .finish_non_exhaustive()
    }
}

impl RestartHub {
    pub fn new(plan: RestartPlan, preflight: Arc<dyn Preflight>) -> Arc<Self> {
        Arc::new(Self {
            plan,
            preflight,
            state: watch::Sender::new(HubState {
                phase: RestartPhase::Idle,
                pending: false,
                stopping: false,
                last_failure: None,
            }),
        })
    }

    pub fn phase(&self) -> RestartPhase {
        self.state.borrow().phase
    }

    pub fn is_pending(&self) -> bool {
        self.state.borrow().pending
    }

    pub fn is_stopping(&self) -> bool {
        self.state.borrow().stopping
    }

    pub fn last_failure(&self) -> Option<String> {
        self.state.borrow().last_failure.clone()
    }

    pub const fn plan(&self) -> &RestartPlan {
        &self.plan
    }

    pub fn preflight(&self) -> &dyn Preflight {
        self.preflight.as_ref()
    }

    pub fn request(&self) -> Result<(), RestartRequestError> {
        let mut answer = Ok(());
        self.state.send_if_modified(|state| {
            answer = if state.stopping {
                Err(RestartRequestError::ShuttingDown)
            } else if state.pending || state.phase != RestartPhase::Idle {
                Err(RestartRequestError::InProgress)
            } else {
                Ok(())
            };
            state.pending |= answer.is_ok();
            answer.is_ok()
        });
        answer
    }

    pub fn mark_stopping(&self) {
        self.state
            .send_if_modified(|state| !std::mem::replace(&mut state.stopping, true));
    }

    pub async fn requested(&self) {
        let mut state = self.state.subscribe();
        let _ = state.wait_for(|state| state.pending).await;
    }

    pub fn set_phase(&self, phase: RestartPhase) -> bool {
        self.state.send_if_modified(|state| {
            if phase <= state.phase {
                return false;
            }
            state.phase = phase;
            state.pending = false;
            true
        })
    }

    pub fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::warn!(phase = ?self.phase(), %reason, "hot restart failed");
        self.state.send_modify(|state| {
            state.phase = RestartPhase::Idle;
            state.pending = false;
            state.last_failure = Some(reason);
        });
    }

    pub fn handle(self: &Arc<Self>) -> Arc<dyn ProxyRestart> {
        Arc::new(RestartHandle {
            hub: Arc::clone(self),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RestartHandle {
    hub: Arc<RestartHub>,
}

impl infrarust_api::__sealed::Sealed for RestartHandle {}

impl ProxyRestart for RestartHandle {
    fn request_restart(&self) -> Result<(), RestartRequestError> {
        self.hub.request()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;
    use std::time::Duration;

    use infrarust_api::BoxFuture;

    use super::*;
    use crate::binary::{BuildInfo, ExeSnapshot, HandoverProbe, RunningBinary};
    use crate::hot_restart::preflight::PreflightError;

    struct Refusing;

    impl Preflight for Refusing {
        fn check<'a>(
            &'a self,
            _plan: &'a RestartPlan,
        ) -> BoxFuture<'a, Result<HandoverProbe, PreflightError>> {
            Box::pin(async { Err(PreflightError::ChildServers(vec!["lobby".into()])) })
        }
    }

    fn hub() -> Arc<RestartHub> {
        let target = PathBuf::from("/opt/infrarust/infrarust");
        let build = BuildInfo {
            version: "2.0.0-beta.6",
            target: "x86_64-unknown-linux-musl",
            official: true,
            docker: false,
        };
        let plan = RestartPlan {
            binary: RunningBinary::new(build, ExeSnapshot::at(target.clone()), Vec::new()),
            target,
        };
        RestartHub::new(plan, Arc::new(Refusing))
    }

    #[test]
    fn a_second_request_is_told_a_restart_is_in_progress() {
        let hub = hub();
        assert_eq!(hub.phase(), RestartPhase::Idle);

        hub.request().unwrap();

        assert!(hub.is_pending());
        assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
        assert!(hub.set_phase(RestartPhase::Preflight));
        assert!(!hub.is_pending());
        assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
        assert!(hub.set_phase(RestartPhase::Freezing));
        assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
    }

    #[test]
    fn a_request_during_a_stop_is_told_the_proxy_shuts_down() {
        let idle = hub();
        idle.mark_stopping();
        assert!(idle.is_stopping());
        assert_eq!(idle.request(), Err(RestartRequestError::ShuttingDown));
        assert!(!idle.is_pending());

        let restarting = hub();
        restarting.request().unwrap();
        restarting.set_phase(RestartPhase::Draining);
        restarting.mark_stopping();
        assert_eq!(restarting.request(), Err(RestartRequestError::ShuttingDown));
    }

    #[test]
    fn a_failed_restart_can_be_requested_again() {
        let hub = hub();
        hub.request().unwrap();
        hub.set_phase(RestartPhase::Preflight);

        hub.fail("lobby runs as a child process");

        assert_eq!(hub.phase(), RestartPhase::Idle);
        assert!(!hub.is_pending());
        assert_eq!(
            hub.last_failure().as_deref(),
            Some("lobby runs as a child process")
        );
        hub.request().unwrap();
        assert!(hub.is_pending());
    }

    #[test]
    fn phases_only_move_forward_until_a_failure() {
        let hub = hub();
        hub.request().unwrap();
        assert!(hub.set_phase(RestartPhase::Closing));
        assert!(!hub.set_phase(RestartPhase::Draining));
        assert!(!hub.set_phase(RestartPhase::Closing));
        assert!(!hub.set_phase(RestartPhase::Idle));
        assert_eq!(hub.phase(), RestartPhase::Closing);
        assert!(hub.set_phase(RestartPhase::HandingOver));
        assert!(RestartPhase::Closing < RestartPhase::HandingOver);
    }

    #[tokio::test]
    async fn requested_wakes_up_once_a_request_is_made() {
        let hub = hub();
        let waiter = tokio::spawn({
            let hub = Arc::clone(&hub);
            async move { hub.requested().await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        hub.request().unwrap();

        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("requested() must wake up")
            .unwrap();
    }

    #[tokio::test]
    async fn requested_is_cancel_safe() {
        let hub = hub();
        let mut cancelled = Box::pin(hub.requested());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut cancelled)
                .await
                .is_err()
        );
        hub.request().unwrap();
        drop(cancelled);

        tokio::time::timeout(Duration::from_secs(5), hub.requested())
            .await
            .expect("a request made while a waiter was dropped stays visible");
        tokio::time::timeout(Duration::from_secs(5), hub.requested())
            .await
            .expect("waiting does not consume the request");

        hub.set_phase(RestartPhase::Preflight);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), hub.requested())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_handle_requests_through_the_hub() {
        let hub = hub();
        let handle = hub.handle();

        handle.request_restart().unwrap();

        assert!(hub.is_pending());
        assert_eq!(
            handle.request_restart(),
            Err(RestartRequestError::InProgress)
        );
        let error = hub.preflight().check(hub.plan()).await.unwrap_err();
        assert!(matches!(error, PreflightError::ChildServers(_)));
    }
}
