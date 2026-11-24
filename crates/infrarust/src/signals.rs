use std::future::Future;
use std::sync::{Arc, OnceLock};
#[cfg(target_os = "linux")]
use std::time::Duration;

use infrarust_core::hot_restart::{RestartHub, RestartPhase};
use tokio_util::sync::CancellationToken;

#[cfg(target_os = "linux")]
const RACE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopSignal {
    Interrupt,
    Terminate,
}

impl StopSignal {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        }
    }

    const fn exit_code(self) -> i32 {
        match self {
            Self::Interrupt => 130,
            Self::Terminate => 143,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    Stop(StopSignal),
    Restart,
}

pub(crate) trait Signals: Send {
    fn next(&mut self) -> impl Future<Output = Option<Signal>> + Send;
}

pub(crate) type HubSlot = Arc<OnceLock<Option<Arc<RestartHub>>>>;

pub(crate) struct Watch {
    pub(crate) shutdown: CancellationToken,
    pub(crate) hub: HubSlot,
}

pub(crate) struct OsSignals {
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    restart: Option<tokio::signal::unix::Signal>,
    #[cfg(target_os = "linux")]
    replay: Option<Replay>,
}

#[cfg(target_os = "linux")]
struct Replay {
    signal: StopSignal,
    deadline: Option<tokio::time::Instant>,
}

impl OsSignals {
    pub(crate) fn install() -> Self {
        #[cfg(target_os = "linux")]
        {
            let (signals, early) =
                infrarust_core::hot_restart::platform::linux::take_over_signals(Self::register);
            signals.replaying(early)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::register()
        }
    }

    fn register() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let terminate = match signal(SignalKind::terminate()) {
                Ok(terminate) => Some(terminate),
                Err(e) => {
                    tracing::error!(error = %e, "failed to install SIGTERM handler; only Ctrl-C will stop the proxy");
                    None
                }
            };
            let restart = match signal(SignalKind::user_defined2()) {
                Ok(restart) => Some(restart),
                Err(e) => {
                    tracing::error!(error = %e, "failed to install SIGUSR2 handler; restart from the console instead");
                    None
                }
            };
            Self {
                terminate,
                restart,
                #[cfg(target_os = "linux")]
                replay: None,
            }
        }
        #[cfg(not(unix))]
        {
            Self {}
        }
    }

    #[cfg(target_os = "linux")]
    fn replaying(
        mut self,
        early: infrarust_core::hot_restart::platform::linux::EarlySignals,
    ) -> Self {
        use infrarust_core::hot_restart::platform::linux::HandoverSignal;

        if early.restart {
            tracing::info!("a restart request arrived during the previous restart and was ignored");
        }
        let Some(stop) = early.stop() else {
            return self;
        };
        let signal = match stop {
            HandoverSignal::Interrupt => StopSignal::Interrupt,
            HandoverSignal::Terminate | HandoverSignal::Restart => StopSignal::Terminate,
        };
        tracing::info!(
            signal = signal.name(),
            "a stop signal arrived during the restart; this image stops now"
        );
        self.replay = Some(Replay {
            signal,
            deadline: early
                .racing(stop)
                .then(|| tokio::time::Instant::now() + RACE),
        });
        self
    }

    async fn receive(&mut self) -> Option<Signal> {
        #[cfg(unix)]
        {
            let terminate = recv(self.terminate.as_mut());
            let restart = recv(self.restart.as_mut());
            tokio::select! {
                biased;
                Some(()) = terminate => Some(Signal::Stop(StopSignal::Terminate)),
                received = tokio::signal::ctrl_c() => received.ok().map(|()| Signal::Stop(StopSignal::Interrupt)),
                Some(()) = restart => Some(Signal::Restart),
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .ok()
                .map(|()| Signal::Stop(StopSignal::Interrupt))
        }
    }
}

#[cfg(unix)]
async fn recv(signal: Option<&mut tokio::signal::unix::Signal>) -> Option<()> {
    match signal {
        Some(signal) => signal.recv().await,
        None => std::future::pending().await,
    }
}

impl Signals for OsSignals {
    async fn next(&mut self) -> Option<Signal> {
        #[cfg(target_os = "linux")]
        if let Some(replay) = self.replay.take() {
            let Some(deadline) = replay.deadline else {
                return Some(Signal::Stop(replay.signal));
            };
            return match tokio::time::timeout_at(deadline, self.receive()).await {
                Ok(Some(Signal::Restart)) => {
                    self.replay = Some(replay);
                    Some(Signal::Restart)
                }
                Ok(received) => received,
                Err(_) => Some(Signal::Stop(replay.signal)),
            };
        }
        self.receive().await
    }
}

pub(crate) async fn watch_signals(
    mut signals: impl Signals,
    watch: Watch,
    exit: impl FnOnce(i32) + Send,
) {
    while let Some(signal) = signals.next().await {
        let stop = match signal {
            Signal::Restart => {
                request_restart(&watch.hub);
                continue;
            }
            Signal::Stop(stop) => stop,
        };
        if let Some(phase) = handing_over(&watch.hub) {
            crate::hot_restart::defer_stop(stop);
            tracing::info!(
                signal = stop.name(),
                ?phase,
                "stop signal received while the sessions are handed over; the next image stops as soon as it starts"
            );
            continue;
        }
        if watch.shutdown.is_cancelled() {
            tracing::warn!(
                signal = stop.name(),
                "shutdown signal received while the proxy is already stopping; exiting now without finishing the shutdown"
            );
            exit(stop.exit_code());
            return;
        }
        tracing::info!(
            signal = stop.name(),
            "shutdown signal received; send it again to exit at once"
        );
        watch.shutdown.cancel();
    }
}

fn handing_over(hub: &HubSlot) -> Option<RestartPhase> {
    let phase = hub.get()?.as_ref()?.phase();
    (phase >= RestartPhase::Closing).then_some(phase)
}

fn request_restart(hub: &HubSlot) {
    match hub.get() {
        None => {
            tracing::info!(
                "SIGUSR2 received while the proxy is starting; the restart request is ignored"
            );
        }
        Some(None) => {
            tracing::warn!(
                "SIGUSR2 received, but hot restart is unavailable here; stop the proxy and start it again"
            );
        }
        Some(Some(hub)) => match hub.request() {
            Ok(()) => tracing::info!("SIGUSR2 received, restarting in place"),
            Err(error) => {
                tracing::warn!(%error, "SIGUSR2 received, the restart request is refused");
            }
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use infrarust_core::binary::{ExeSnapshot, RunningBinary};
    use infrarust_core::hot_restart::{BinaryPreflight, RestartPlan};

    use super::*;

    struct Scripted(VecDeque<Signal>);

    impl Signals for Scripted {
        fn next(&mut self) -> impl Future<Output = Option<Signal>> + Send {
            std::future::ready(self.0.pop_front())
        }
    }

    fn hub() -> Arc<RestartHub> {
        let binary = RunningBinary::new(
            crate::BUILD,
            ExeSnapshot::at(PathBuf::from("/nonexistent/infrarust")),
            Vec::new(),
        );
        RestartHub::new(
            RestartPlan {
                target: binary.exe.path().to_path_buf(),
                binary,
            },
            Arc::new(BinaryPreflight),
        )
    }

    fn slot(hub: Option<Option<Arc<RestartHub>>>) -> HubSlot {
        let slot = HubSlot::default();
        if let Some(hub) = hub {
            slot.set(hub).unwrap();
        }
        slot
    }

    async fn watch(signals: &[Signal], shutdown: &CancellationToken, hub: HubSlot) -> Option<i32> {
        let exited = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&exited);
        watch_signals(
            Scripted(signals.iter().copied().collect()),
            Watch {
                shutdown: shutdown.clone(),
                hub,
            },
            move |code| *seen.lock().unwrap() = Some(code),
        )
        .await;
        *exited.lock().unwrap()
    }

    const TERM: Signal = Signal::Stop(StopSignal::Terminate);
    const INT: Signal = Signal::Stop(StopSignal::Interrupt);

    #[tokio::test]
    async fn the_first_signal_starts_the_shutdown_and_the_second_exits_at_once() {
        let shutdown = CancellationToken::new();
        let code = watch(&[TERM, INT], &shutdown, slot(None)).await;
        assert!(shutdown.is_cancelled());
        assert_eq!(code, Some(130));
    }

    #[tokio::test]
    async fn one_signal_only_starts_the_shutdown() {
        let shutdown = CancellationToken::new();
        assert_eq!(watch(&[TERM], &shutdown, slot(None)).await, None);
        assert!(shutdown.is_cancelled());
    }

    #[tokio::test]
    async fn a_signal_while_the_proxy_is_already_stopping_exits_at_once() {
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        assert_eq!(
            watch(&[TERM], &shutdown, slot(Some(None))).await,
            Some(143),
            "a stop from the console counts as the first request"
        );
    }

    #[tokio::test]
    async fn a_restart_signal_asks_the_hub_for_a_restart() {
        let hub = hub();
        let shutdown = CancellationToken::new();

        assert_eq!(
            watch(
                &[Signal::Restart],
                &shutdown,
                slot(Some(Some(Arc::clone(&hub))))
            )
            .await,
            None
        );

        assert!(hub.is_pending());
        assert!(!shutdown.is_cancelled());
    }

    #[tokio::test]
    async fn a_restart_signal_without_a_hub_changes_nothing() {
        let shutdown = CancellationToken::new();
        for hub in [slot(None), slot(Some(None))] {
            assert_eq!(watch(&[Signal::Restart], &shutdown, hub).await, None);
        }
        assert!(!shutdown.is_cancelled());
    }

    #[tokio::test]
    async fn a_stop_before_the_sessions_close_is_a_normal_stop() {
        let hub = hub();
        hub.request().unwrap();
        hub.set_phase(RestartPhase::Draining);
        let shutdown = CancellationToken::new();

        let code = watch(&[TERM, TERM], &shutdown, slot(Some(Some(hub)))).await;

        assert!(shutdown.is_cancelled());
        assert_eq!(code, Some(143));
    }

    #[tokio::test]
    async fn a_stop_while_the_sessions_are_handed_over_is_deferred() {
        let hub = hub();
        hub.request().unwrap();
        hub.set_phase(RestartPhase::Closing);
        let shutdown = CancellationToken::new();
        shutdown.cancel();

        let code = watch(&[TERM, INT, TERM], &shutdown, slot(Some(Some(hub)))).await;

        assert_eq!(code, None, "a deferred stop never exits the old image");
    }
}
