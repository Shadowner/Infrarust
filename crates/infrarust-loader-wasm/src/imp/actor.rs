use std::any::Any;
use std::fmt;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use infrarust_api::event::BoxFuture;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::instrument::WithSubscriber;
use wasmtime::Store;

use crate::bindings::Plugin as PluginBindings;
use crate::config::SandboxLimits;
use crate::consts::QUEUE_FULL_WARN_INTERVAL;
use crate::store_state::PluginStoreState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallFailure {
    Stopped,
    QueueFull,
    Poisoned,
    Trapped(String),
    Dropped,
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => f.write_str("the plugin instance is stopped"),
            Self::QueueFull => f.write_str("the plugin call queue is full"),
            Self::Poisoned => f.write_str("the plugin instance is poisoned"),
            Self::Trapped(reason) => write!(f, "the guest trapped: {reason}"),
            Self::Dropped => f.write_str("the call was dropped before it completed"),
        }
    }
}

trait GuestCall: Send {
    fn caller_gone(&self) -> bool;

    fn refuse(self: Box<Self>, failure: CallFailure);

    fn run<'a>(
        self: Box<Self>,
        store: &'a mut Store<PluginStoreState>,
        bindings: &'a PluginBindings,
    ) -> BoxFuture<'a, wasmtime::Result<()>>;
}

struct TypedCall<T, F> {
    reply: oneshot::Sender<Result<T, CallFailure>>,
    call: F,
}

impl<T, F> GuestCall for TypedCall<T, F>
where
    T: Send + 'static,
    F: for<'a> FnOnce(
            &'a mut Store<PluginStoreState>,
            &'a PluginBindings,
        ) -> BoxFuture<'a, wasmtime::Result<T>>
        + Send
        + 'static,
{
    fn caller_gone(&self) -> bool {
        self.reply.is_closed()
    }

    fn refuse(self: Box<Self>, failure: CallFailure) {
        let _ = self.reply.send(Err(failure));
    }

    fn run<'a>(
        self: Box<Self>,
        store: &'a mut Store<PluginStoreState>,
        bindings: &'a PluginBindings,
    ) -> BoxFuture<'a, wasmtime::Result<()>> {
        let TypedCall { reply, call } = *self;
        let pending = call(store, bindings);
        Box::pin(async move {
            match pending.await {
                Ok(value) => {
                    let _ = reply.send(Ok(value));
                    Ok(())
                }
                Err(trap) => {
                    let _ = reply.send(Err(CallFailure::Trapped(trap.to_string())));
                    Err(trap)
                }
            }
        })
    }
}

struct Job {
    op: &'static str,
    last: bool,
    call: Box<dyn GuestCall>,
}

impl Job {
    fn new<T, F>(
        op: &'static str,
        last: bool,
        call: F,
    ) -> (Self, oneshot::Receiver<Result<T, CallFailure>>)
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let job = Self {
            op,
            last,
            call: Box::new(TypedCall { reply, call }),
        };
        (job, answer)
    }
}

struct ActorInfo {
    plugin_id: String,
    capacity: usize,
    started: Instant,
    next_full_warning_ms: AtomicU64,
    suppressed_full_warnings: AtomicU64,
}

impl ActorInfo {
    fn new(plugin_id: String, capacity: usize) -> Self {
        Self {
            plugin_id,
            capacity,
            started: Instant::now(),
            next_full_warning_ms: AtomicU64::new(0),
            suppressed_full_warnings: AtomicU64::new(0),
        }
    }

    fn warn_queue_full(&self, op: &'static str) {
        let now = millis(self.started.elapsed());
        let next = self.next_full_warning_ms.load(Ordering::Relaxed);
        let due = now >= next
            && self
                .next_full_warning_ms
                .compare_exchange(
                    next,
                    now.saturating_add(millis(QUEUE_FULL_WARN_INTERVAL)),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok();
        if due {
            let suppressed = self.suppressed_full_warnings.swap(0, Ordering::Relaxed);
            tracing::warn!(plugin = %self.plugin_id, op, capacity = self.capacity, suppressed,
                "wasm plugin call queue is full; refusing the call");
        } else {
            self.suppressed_full_warnings
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Clone)]
pub(crate) struct InstanceRef {
    jobs: mpsc::WeakSender<Job>,
    info: Arc<ActorInfo>,
}

impl InstanceRef {
    pub(crate) fn detached() -> Self {
        let (jobs, _) = mpsc::channel(1);
        Self {
            jobs: jobs.downgrade(),
            info: Arc::new(ActorInfo::new(String::new(), 1)),
        }
    }

    pub(crate) async fn call<T, F>(&self, op: &'static str, call: F) -> Result<T, CallFailure>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let Some(jobs) = self.jobs.upgrade() else {
            return Err(CallFailure::Stopped);
        };
        let (job, answer) = Job::new(op, false, call);
        match jobs.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.info.warn_queue_full(op);
                return Err(CallFailure::QueueFull);
            }
            Err(TrySendError::Closed(_)) => return Err(CallFailure::Stopped),
        }
        drop(jobs);
        answer.await.unwrap_or(Err(CallFailure::Dropped))
    }
}

pub(crate) struct PluginActor {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    instance: InstanceRef,
    stopping: Arc<AtomicBool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl PluginActor {
    pub(crate) fn spawn(
        mut store: Store<PluginStoreState>,
        bindings: PluginBindings,
        sandbox: &SandboxLimits,
    ) -> Arc<Self> {
        let (jobs, queue) = mpsc::channel(sandbox.queue_capacity);
        let info = Arc::new(ActorInfo::new(
            store.data().plugin_id.clone(),
            sandbox.queue_capacity,
        ));
        let instance = InstanceRef {
            jobs: jobs.downgrade(),
            info,
        };
        store.data_mut().set_instance_ref(instance.clone());
        let stopping = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(
            run(
                store,
                bindings,
                queue,
                Arc::clone(&stopping),
                sandbox.max_call_duration,
            )
            .with_current_subscriber(),
        );
        Arc::new(Self {
            jobs: Mutex::new(Some(jobs)),
            instance,
            stopping,
            task: Mutex::new(Some(task)),
        })
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.instance.info.plugin_id
    }

    pub(crate) async fn call_lifecycle<T, F>(
        &self,
        op: &'static str,
        last: bool,
        call: F,
    ) -> Result<T, CallFailure>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let jobs = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(jobs) = jobs else {
            return Err(CallFailure::Stopped);
        };
        let (job, answer) = Job::new(op, last, call);
        if jobs.send(job).await.is_err() {
            return Err(CallFailure::Stopped);
        }
        drop(jobs);
        answer.await.unwrap_or(Err(CallFailure::Dropped))
    }

    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        let jobs = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(jobs);
    }

    pub(crate) async fn shutdown(&self) {
        self.stop();
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task
            && let Err(e) = task.await
        {
            tracing::error!(plugin = %self.plugin_id(), error = %e,
                "wasm plugin task ended abnormally");
        }
    }
}

impl Drop for PluginActor {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn run(
    mut store: Store<PluginStoreState>,
    bindings: PluginBindings,
    mut queue: mpsc::Receiver<Job>,
    stopping: Arc<AtomicBool>,
    max_call_duration: Duration,
) {
    while let Some(job) = queue.recv().await {
        if stopping.load(Ordering::Acquire) {
            job.call.refuse(CallFailure::Stopped);
            break;
        }
        let last = job.last;
        execute(&mut store, &bindings, job, max_call_duration).await;
        if last {
            break;
        }
    }
    tracing::debug!(plugin = %store.data().plugin_id, "wasm plugin task stopped");
}

async fn execute(
    store: &mut Store<PluginStoreState>,
    bindings: &PluginBindings,
    job: Job,
    max_call_duration: Duration,
) {
    let Job { op, call, .. } = job;
    if call.caller_gone() {
        tracing::debug!(plugin = %store.data().plugin_id, op,
            "skipping a queued wasm guest call: its caller stopped waiting");
        return;
    }
    if store.data_mut().is_poisoned() {
        call.refuse(CallFailure::Poisoned);
        return;
    }
    store.data_mut().reset_epoch_budget();
    store.data_mut().begin_call(op);
    let running = AssertUnwindSafe(call.run(store, bindings)).catch_unwind();
    let outcome = tokio::time::timeout(max_call_duration, running).await;
    let state = store.data_mut();
    match outcome {
        Ok(Ok(Ok(()))) => state.end_call(),
        Ok(Ok(Err(trap))) => {
            state.end_call();
            tracing::error!(plugin = %state.plugin_id, op, error = %trap,
                "wasm guest trapped; poisoning instance");
            state.set_poisoned();
        }
        Ok(Err(payload)) => {
            tracing::error!(plugin = %state.plugin_id, op, panic = %panic_message(payload.as_ref()),
                "wasm guest call panicked in a host function; abandoning it");
        }
        Err(_) => {
            tracing::error!(plugin = %state.plugin_id, op, limit = ?max_call_duration,
                "wasm guest call exceeded max_call_duration; abandoning it");
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "non-string panic payload"
    }
}
