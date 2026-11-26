use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use infrarust_api::services::proxy_control::{
    ProxyRestart, RestartCancelError, RestartOptions, RestartRequestError, ScheduledRestart,
};
use tokio::sync::watch;

use super::announce;
use super::platform::RestartPlan;
use super::preflight::Preflight;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RestartPhase {
    Idle,
    Preflight,
    Announcing,
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
    refusal: Option<String>,
    last_failure: Option<String>,
    options: RestartOptions,
    scheduled: Option<SystemTime>,
    cancelled: bool,
}

impl HubState {
    fn clear_request(&mut self) -> bool {
        self.cancelled = false;
        self.scheduled = None;
        self.options = RestartOptions::default();
        std::mem::take(&mut self.pending)
    }

    fn drop_pending(&mut self, reason: &str) -> bool {
        if self.phase != RestartPhase::Idle || !self.clear_request() {
            return false;
        }
        self.last_failure = Some(reason.to_string());
        true
    }
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
                refusal: None,
                last_failure: None,
                options: RestartOptions::default(),
                scheduled: None,
                cancelled: false,
            }),
        })
    }

    pub fn phase(&self) -> RestartPhase {
        self.state.borrow().phase
    }

    pub fn is_pending(&self) -> bool {
        self.state.borrow().pending
    }

    #[cfg(test)]
    fn is_stopping(&self) -> bool {
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
        self.request_with(RestartOptions::default())
    }

    pub fn request_with(&self, options: RestartOptions) -> Result<(), RestartRequestError> {
        announce::validate(&options)?;
        let mut answer = Ok(());
        self.state.send_if_modified(|state| {
            answer = if state.stopping {
                Err(RestartRequestError::ShuttingDown)
            } else if let Some(refusal) = &state.refusal {
                Err(RestartRequestError::Unsupported(refusal.clone()))
            } else if state.pending || state.phase != RestartPhase::Idle {
                Err(RestartRequestError::InProgress)
            } else {
                Ok(())
            };
            if answer.is_ok() {
                state.clear_request();
                state.pending = true;
                state.options = options;
            }
            answer.is_ok()
        });
        answer
    }

    pub fn begin_preflight(&self) -> Option<RestartOptions> {
        let mut options = None;
        self.state.send_if_modified(|state| {
            if !state.pending || state.phase != RestartPhase::Idle {
                return false;
            }
            state.phase = RestartPhase::Preflight;
            state.pending = false;
            options = Some(std::mem::take(&mut state.options));
            true
        });
        options
    }

    pub fn announce_until(&self, at: SystemTime) {
        self.state.send_if_modified(|state| {
            if state.phase >= RestartPhase::Announcing {
                return false;
            }
            state.phase = RestartPhase::Announcing;
            state.pending = false;
            state.scheduled = Some(at);
            true
        });
    }

    pub fn begin_draining(&self) -> bool {
        self.state.send_if_modified(|state| {
            let cancellable = matches!(
                state.phase,
                RestartPhase::Preflight | RestartPhase::Announcing
            );
            if state.cancelled || !cancellable {
                return false;
            }
            state.phase = RestartPhase::Draining;
            state.clear_request();
            true
        })
    }

    pub fn scheduled(&self) -> Option<ScheduledRestart> {
        self.state
            .borrow()
            .scheduled
            .map(|at| ScheduledRestart { at })
    }

    pub fn cancel(&self) -> Result<(), RestartCancelError> {
        let mut answer = Ok(());
        self.state.send_if_modified(|state| {
            answer = if state.phase > RestartPhase::Announcing {
                Err(RestartCancelError::TooLate)
            } else if state.cancelled || (state.phase == RestartPhase::Idle && !state.pending) {
                Err(RestartCancelError::NothingScheduled)
            } else {
                Ok(())
            };
            state.cancelled |= answer.is_ok();
            answer.is_ok()
        });
        answer
    }

    pub async fn cancelled(&self) {
        let mut state = self.state.subscribe();
        let _ = state.wait_for(|state| state.cancelled).await;
    }

    pub fn mark_stopping(&self) {
        self.state.send_if_modified(|state| {
            let dropped = state.drop_pending("the proxy is stopping");
            !std::mem::replace(&mut state.stopping, true) || dropped
        });
    }

    pub fn refuse_requests(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.state.send_modify(|state| {
            if state.drop_pending(&reason) {
                tracing::warn!(%reason, "a pending hot restart request is dropped");
            }
            state.refusal = Some(reason);
        });
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
            if phase > RestartPhase::Announcing {
                state.clear_request();
            }
            true
        })
    }

    pub fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::warn!(phase = ?self.phase(), %reason, "hot restart failed");
        self.state.send_modify(|state| {
            state.phase = RestartPhase::Idle;
            state.clear_request();
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
    fn request_restart_with(&self, options: RestartOptions) -> Result<(), RestartRequestError> {
        self.hub.request_with(options)
    }

    fn cancel_restart(&self) -> Result<(), RestartCancelError> {
        self.hub.cancel()
    }

    fn scheduled_restart(&self) -> Option<ScheduledRestart> {
        self.hub.scheduled()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;
    use std::time::Duration;

    use infrarust_api::BoxFuture;
    use infrarust_config::{MAX_RESTART_DELAY, MAX_RESTART_MESSAGE};

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

    fn options(delay: u64, message: &str) -> RestartOptions {
        RestartOptions {
            delay: Some(Duration::from_secs(delay)),
            message: Some(message.to_string()),
        }
    }

    fn at() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    fn announcing(options: RestartOptions) -> Arc<RestartHub> {
        let hub = hub();
        hub.request_with(options).unwrap();
        hub.begin_preflight().unwrap();
        hub.announce_until(at());
        hub
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
    fn a_hub_refusing_requests_answers_unsupported_and_drops_a_pending_one() {
        let hub = hub();
        hub.request().unwrap();

        hub.refuse_requests("the proxy runs without its restart loop");

        assert!(!hub.is_pending());
        assert_eq!(
            hub.last_failure().as_deref(),
            Some("the proxy runs without its restart loop")
        );
        assert_eq!(
            hub.request(),
            Err(RestartRequestError::Unsupported(
                "the proxy runs without its restart loop".into()
            ))
        );
        hub.mark_stopping();
        assert_eq!(hub.request(), Err(RestartRequestError::ShuttingDown));
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

    #[test]
    fn a_request_hands_its_options_to_the_restart_once() {
        let hub = hub();
        assert_eq!(hub.begin_preflight(), None);
        assert_eq!(hub.phase(), RestartPhase::Idle);

        hub.request_with(options(30, "Back soon")).unwrap();
        assert_eq!(
            hub.request_with(options(5, "Other")),
            Err(RestartRequestError::InProgress)
        );

        assert_eq!(hub.begin_preflight(), Some(options(30, "Back soon")));
        assert_eq!(hub.phase(), RestartPhase::Preflight);
        assert!(!hub.is_pending());
        assert_eq!(hub.begin_preflight(), None);
    }

    #[test]
    fn the_options_of_a_failed_request_do_not_reach_the_next_one() {
        let hub = hub();
        hub.request_with(options(30, "Back soon")).unwrap();
        hub.fail("refused");

        hub.request().unwrap();

        assert_eq!(hub.begin_preflight(), Some(RestartOptions::default()));
    }

    #[test]
    fn options_over_the_limits_are_refused_before_anything_is_scheduled() {
        let hub = hub();
        let too_far = MAX_RESTART_DELAY.as_secs() + 1;
        assert!(matches!(
            hub.request_with(options(too_far, "Later")),
            Err(RestartRequestError::Invalid(_))
        ));
        let too_long = "x".repeat(MAX_RESTART_MESSAGE + 1);
        assert!(matches!(
            hub.request_with(options(0, &too_long)),
            Err(RestartRequestError::Invalid(_))
        ));
        assert!(!hub.is_pending());
        assert_eq!(hub.phase(), RestartPhase::Idle);
    }

    #[test]
    fn the_schedule_is_known_while_the_restart_is_announced() {
        let hub = hub();
        hub.request().unwrap();
        hub.begin_preflight().unwrap();
        assert_eq!(hub.scheduled(), None);

        hub.announce_until(at());

        assert_eq!(hub.phase(), RestartPhase::Announcing);
        assert_eq!(hub.scheduled(), Some(ScheduledRestart { at: at() }));
        assert_eq!(hub.request(), Err(RestartRequestError::InProgress));
        assert!(hub.begin_draining());
        assert_eq!(hub.scheduled(), None);
    }

    #[tokio::test]
    async fn the_announcement_and_its_date_are_published_together() {
        let hub = hub();
        hub.request().unwrap();
        hub.begin_preflight().unwrap();
        let mut seen = hub.state.subscribe();

        hub.announce_until(at());

        seen.changed().await.unwrap();
        let state = seen.borrow_and_update().clone();
        assert_eq!(state.phase, RestartPhase::Announcing);
        assert_eq!(state.scheduled, Some(at()));
        assert!(!seen.has_changed().unwrap());
    }

    #[test]
    fn nothing_is_cancelled_while_no_restart_is_requested() {
        let hub = hub();
        assert_eq!(hub.cancel(), Err(RestartCancelError::NothingScheduled));
        assert!(!hub.begin_draining());
        assert_eq!(hub.phase(), RestartPhase::Idle);
    }

    #[test]
    fn a_pending_request_is_cancelled_only_once() {
        let hub = hub();
        hub.request().unwrap();

        assert_eq!(hub.cancel(), Ok(()));
        assert_eq!(hub.cancel(), Err(RestartCancelError::NothingScheduled));

        hub.fail("the restart was cancelled");
        assert_eq!(hub.cancel(), Err(RestartCancelError::NothingScheduled));
    }

    #[test]
    fn a_cancelled_announcement_leaves_nothing_behind() {
        let hub = announcing(options(30, "Back soon"));

        assert_eq!(hub.cancel(), Ok(()));
        hub.fail("the restart was cancelled");

        assert_eq!(hub.phase(), RestartPhase::Idle);
        assert_eq!(hub.scheduled(), None);
        hub.request().unwrap();
        assert_eq!(hub.begin_preflight(), Some(RestartOptions::default()));
        assert!(hub.begin_draining(), "a new request starts clean");
    }

    #[test]
    fn a_cancel_answered_ok_keeps_the_restart_from_draining() {
        for hub in [announcing(RestartOptions::default()), {
            let hub = hub();
            hub.request().unwrap();
            hub.begin_preflight().unwrap();
            hub
        }] {
            assert_eq!(hub.cancel(), Ok(()));

            assert!(!hub.begin_draining());

            assert!(hub.phase() < RestartPhase::Draining);
            hub.fail("the restart was cancelled");
            assert_eq!(hub.phase(), RestartPhase::Idle);
        }
    }

    #[test]
    fn a_cancel_after_the_proxy_stops_accepting_is_too_late() {
        let hub = announcing(RestartOptions::default());

        assert!(hub.begin_draining());

        assert_eq!(hub.phase(), RestartPhase::Draining);
        assert_eq!(hub.cancel(), Err(RestartCancelError::TooLate));
        assert!(!hub.begin_draining());
        assert!(hub.set_phase(RestartPhase::Closing));
        assert_eq!(hub.cancel(), Err(RestartCancelError::TooLate));
    }

    #[test]
    fn a_stop_or_a_refusal_drops_the_pending_request_with_its_options() {
        let stopping = hub();
        stopping.request_with(options(30, "Back soon")).unwrap();
        stopping.mark_stopping();
        assert!(!stopping.is_pending());
        assert_eq!(
            stopping.last_failure().as_deref(),
            Some("the proxy is stopping")
        );
        assert_eq!(stopping.begin_preflight(), None);
        assert_eq!(stopping.cancel(), Err(RestartCancelError::NothingScheduled));

        let refusing = hub();
        refusing.request_with(options(30, "Back soon")).unwrap();
        refusing.cancel().unwrap();
        refusing.refuse_requests("no restart loop");
        assert_eq!(refusing.begin_preflight(), None);
        assert_eq!(refusing.cancel(), Err(RestartCancelError::NothingScheduled));
    }

    #[test]
    fn a_stop_during_a_restart_leaves_its_state_to_the_restart() {
        let hub = announcing(RestartOptions::default());

        hub.mark_stopping();

        assert_eq!(hub.phase(), RestartPhase::Announcing);
        assert_eq!(hub.scheduled(), Some(ScheduledRestart { at: at() }));
        assert_eq!(hub.last_failure(), None);
        hub.fail("the proxy is stopping");
        assert_eq!(hub.scheduled(), None);
        assert_eq!(hub.request(), Err(RestartRequestError::ShuttingDown));
    }

    #[tokio::test]
    async fn a_cancellation_wakes_whoever_waits_for_it() {
        let hub = hub();
        hub.request().unwrap();
        hub.set_phase(RestartPhase::Preflight);
        let waiting = {
            let hub = Arc::clone(&hub);
            tokio::spawn(async move { hub.cancelled().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        hub.cancel().unwrap();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn the_handle_exposes_the_cancellation_and_the_schedule() {
        let hub = hub();
        let handle = hub.handle();
        assert_eq!(handle.scheduled_restart(), None);
        assert_eq!(
            handle.cancel_restart(),
            Err(RestartCancelError::NothingScheduled)
        );
        handle.request_restart_with(options(10, "Soon")).unwrap();
        assert_eq!(handle.cancel_restart(), Ok(()));
        assert_eq!(hub.begin_preflight(), Some(options(10, "Soon")));
        hub.announce_until(at());
        assert_eq!(
            handle.scheduled_restart(),
            Some(ScheduledRestart { at: at() })
        );
    }
}
