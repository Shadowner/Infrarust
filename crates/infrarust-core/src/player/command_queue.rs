use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_api::command::{CommandSource, Suggestion};
use infrarust_api::event::BoxFuture;
use infrarust_api::message::ProxyMessage;
use infrarust_api::player::Player;

use super::PlayerSession;
use crate::event_bus::diagnostic::panic_message;
use crate::services::command_manager::{
    CommandManagerImpl, Completion, DispatchOutcome, Invocation, Prepared,
};

const QUEUE_CAPACITY: usize = 16;

const COMMANDS_BUSY: &str =
    "You are sending commands faster than they can run. Wait for the last one to finish.";

impl PlayerSession {
    pub(crate) fn dispatch_command(
        self: &Arc<Self>,
        commands: &CommandManagerImpl,
        input: &str,
    ) -> DispatchOutcome {
        let source = CommandSource::Player(Arc::clone(self) as Arc<dyn Player>);
        match commands.prepare(source, input) {
            Prepared::Unknown => DispatchOutcome::Unknown,
            Prepared::Denied => DispatchOutcome::Denied,
            Prepared::Ready(invocation) => {
                self.run_command(invocation);
                DispatchOutcome::Executed
            }
        }
    }

    fn run_command(&self, invocation: Invocation) {
        let job = Queued {
            owner: invocation.owner().map(str::to_owned),
            label: invocation.label().to_owned(),
            run: invocation.run(),
        };
        match self.typed_commands.submit(job) {
            Ok(()) => {}
            Err(Refused::Full) => {
                tracing::debug!(player = %self.profile.username,
                    "refusing a command: too many of the player's commands are waiting to run");
                let _ = self.send_message(ProxyMessage::error(COMMANDS_BUSY));
            }
            Err(Refused::Closed) => {
                tracing::debug!(player = %self.profile.username,
                    "dropping a command typed while the player's session ends");
            }
        }
    }

    pub(crate) fn run_completion(
        &self,
        completion: Completion,
        reply: impl FnOnce(Vec<Suggestion>) + Send + 'static,
    ) {
        let owner = completion.owner().map(str::to_owned);
        let label = completion.label().to_owned();
        let suggesting = completion.run();
        let job = Queued {
            owner,
            label,
            run: Box::pin(async move { reply(suggesting.await) }),
        };
        if let Err(refused) = self.typed_commands.submit(job) {
            tracing::debug!(player = %self.profile.username, ?refused,
                "dropping a tab completion request");
        }
    }

    pub(crate) async fn end_commands(&self) {
        self.typed_commands.close().await;
    }
}

pub(crate) struct Queued {
    pub(crate) owner: Option<String>,
    pub(crate) label: String,
    pub(crate) run: BoxFuture<'static, ()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    Full,
    Closed,
}

enum State {
    Idle,
    Running(Worker),
    Closed,
}

struct Worker {
    jobs: mpsc::Sender<Queued>,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl Worker {
    fn spawn(player: String) -> Self {
        let (jobs, queue) = mpsc::channel(QUEUE_CAPACITY);
        let stop = CancellationToken::new();
        let task = tokio::spawn(work(player, queue, stop.clone()));
        Self { jobs, stop, task }
    }
}

pub(crate) struct CommandQueue {
    player: String,
    state: Mutex<State>,
}

impl CommandQueue {
    pub(crate) fn new(player: impl Into<String>) -> Self {
        Self {
            player: player.into(),
            state: Mutex::new(State::Idle),
        }
    }

    pub(crate) fn submit(&self, job: Queued) -> Result<(), Refused> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*state, State::Idle) {
            *state = State::Running(Worker::spawn(self.player.clone()));
        }
        let State::Running(worker) = &*state else {
            return Err(Refused::Closed);
        };
        worker.jobs.try_send(job).map_err(|e| match e {
            TrySendError::Full(_) => Refused::Full,
            TrySendError::Closed(_) => Refused::Closed,
        })
    }

    pub(crate) fn stop(&self) -> Option<JoinHandle<()>> {
        let state = std::mem::replace(
            &mut *self.state.lock().unwrap_or_else(PoisonError::into_inner),
            State::Closed,
        );
        match state {
            State::Running(worker) => {
                worker.stop.cancel();
                Some(worker.task)
            }
            State::Idle | State::Closed => None,
        }
    }

    pub(crate) async fn close(&self) {
        if let Some(task) = self.stop()
            && let Err(e) = task.await
        {
            tracing::error!(player = %self.player, error = %e, "the player's command task ended abnormally");
        }
    }
}

impl Drop for CommandQueue {
    fn drop(&mut self) {
        drop(self.stop());
    }
}

async fn work(player: String, mut queue: mpsc::Receiver<Queued>, stop: CancellationToken) {
    loop {
        let job = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            job = queue.recv() => match job {
                Some(job) => job,
                None => return,
            },
        };
        let Queued { owner, label, run } = job;
        let plugin = owner.as_deref().unwrap_or("infrarust");
        tokio::select! {
            biased;
            () = stop.cancelled() => {
                tracing::debug!(%player, plugin, command = %label,
                    "the player's session ended; cancelling the command it was still running");
                return;
            }
            outcome = AssertUnwindSafe(run).catch_unwind() => {
                if let Err(payload) = outcome {
                    tracing::error!(%player, plugin, command = %label,
                        panic = %panic_message(payload.as_ref()),
                        "command handler panicked; the player's next command still runs");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::{Notify, oneshot};

    use super::*;

    fn job(run: impl Future<Output = ()> + Send + 'static) -> Queued {
        Queued {
            owner: Some("tester".into()),
            label: "test".into(),
            run: Box::pin(run),
        }
    }

    #[tokio::test]
    async fn jobs_run_one_after_the_other_in_order() {
        let queue = CommandQueue::new("Steve");
        let gate = Arc::new(Notify::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let first = {
            let gate = Arc::clone(&gate);
            let tx = tx.clone();
            job(async move {
                tx.send("first start").unwrap();
                gate.notified().await;
                tx.send("first end").unwrap();
            })
        };
        queue.submit(first).unwrap();
        queue
            .submit(job(async move {
                tx.send("second").unwrap();
            }))
            .unwrap();

        assert_eq!(rx.recv().await, Some("first start"));
        gate.notify_one();
        assert_eq!(rx.recv().await, Some("first end"));
        assert_eq!(rx.recv().await, Some("second"));
        queue.close().await;
    }

    #[tokio::test]
    async fn a_panicking_job_does_not_stop_the_next_one() {
        let queue = CommandQueue::new("Steve");
        let (tx, rx) = oneshot::channel();
        queue.submit(job(async { panic!("boom") })).unwrap();
        queue
            .submit(job(async move {
                tx.send(()).unwrap();
            }))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("the next job runs")
            .unwrap();
        queue.close().await;
    }

    struct Dropped(Option<oneshot::Sender<()>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    #[tokio::test]
    async fn closing_cancels_the_running_job_and_refuses_new_ones() {
        let queue = CommandQueue::new("Steve");
        let (started_tx, started) = oneshot::channel();
        let (dropped_tx, dropped) = oneshot::channel();
        let guard = Dropped(Some(dropped_tx));
        queue
            .submit(job(async move {
                let _guard = guard;
                started_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            }))
            .unwrap();
        started.await.unwrap();

        queue.close().await;

        dropped.await.expect("the running job was dropped");
        assert_eq!(queue.submit(job(async {})).err(), Some(Refused::Closed));
    }

    #[tokio::test]
    async fn a_full_queue_refuses_the_job() {
        let queue = CommandQueue::new("Steve");
        let (started_tx, started) = oneshot::channel();
        queue
            .submit(job(async move {
                started_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            }))
            .unwrap();
        started.await.unwrap();
        for _ in 0..QUEUE_CAPACITY {
            queue.submit(job(async {})).unwrap();
        }
        assert_eq!(queue.submit(job(async {})).err(), Some(Refused::Full));
        queue.close().await;
    }
}
