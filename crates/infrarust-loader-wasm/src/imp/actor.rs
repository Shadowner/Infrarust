use std::fmt;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use infrarust_api::event::BoxFuture;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::instrument::WithSubscriber;
use wasmtime::Store;

use crate::bindings::Plugin as PluginBindings;
use crate::chain::CallChain;
use crate::config::SandboxLimits;
use crate::consts::{GUEST_WARNING_BURST, GUEST_WARNING_INTERVAL, QUEUE_FULL_WARN_INTERVAL};
use crate::deadline::Deadline;
use crate::error::WasmLoaderError;
use crate::instance::InstanceFactory;
use crate::rate_limit::SharedRateLimit;
use crate::store_state::PluginStoreState;
use crate::supervisor::Supervisor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallFailure {
    Stopped,
    QueueFull,
    Failed,
    Quarantined,
    Replaced,
    Expired,
    Trapped(String),
    Abandoned(String),
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallKind {
    Event,
    Callback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    Call,
    Enable,
    Disable,
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => f.write_str("the plugin instance is stopped"),
            Self::QueueFull => f.write_str("the plugin call queue is full"),
            Self::Failed => f.write_str("the plugin failed before it was enabled"),
            Self::Quarantined => f.write_str("the plugin is quarantined after repeated faults"),
            Self::Replaced => {
                f.write_str("the plugin instance the call was meant for was replaced")
            }
            Self::Expired => f.write_str("the call's deadline passed while it was queued"),
            Self::Trapped(reason) => write!(f, "the guest trapped: {reason}"),
            Self::Abandoned(reason) => write!(f, "the call was abandoned: {reason}"),
            Self::Dropped => f.write_str("the call was dropped before it completed"),
        }
    }
}

pub(crate) trait GuestCall: Send {
    fn caller_gone(&self) -> bool;

    fn run<'a>(
        &'a mut self,
        store: &'a mut Store<PluginStoreState>,
        bindings: &'a PluginBindings,
    ) -> BoxFuture<'a, wasmtime::Result<()>>;

    fn answer(self: Box<Self>);

    fn refuse(self: Box<Self>, failure: CallFailure);
}

struct TypedCall<T, F> {
    reply: oneshot::Sender<Result<T, CallFailure>>,
    call: Option<F>,
    value: Option<T>,
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

    fn run<'a>(
        &'a mut self,
        store: &'a mut Store<PluginStoreState>,
        bindings: &'a PluginBindings,
    ) -> BoxFuture<'a, wasmtime::Result<()>> {
        let call = self.call.take();
        let value = &mut self.value;
        Box::pin(async move {
            let call =
                call.ok_or_else(|| wasmtime::Error::msg("a guest call can only run once"))?;
            *value = Some(call(store, bindings).await?);
            Ok(())
        })
    }

    fn answer(self: Box<Self>) {
        let TypedCall { reply, value, .. } = *self;
        let _ = reply.send(value.ok_or(CallFailure::Dropped));
    }

    fn refuse(self: Box<Self>, failure: CallFailure) {
        let _ = self.reply.send(Err(failure));
    }
}

pub(crate) struct Job {
    pub(crate) op: &'static str,
    pub(crate) kind: JobKind,
    pub(crate) deadline: Option<Deadline>,
    pub(crate) generation: Option<u64>,
    pub(crate) chain: CallChain,
    pub(crate) call: Box<dyn GuestCall>,
}

impl Job {
    fn new<T, F>(
        op: &'static str,
        kind: JobKind,
        deadline: Option<Deadline>,
        generation: Option<u64>,
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
            kind,
            deadline,
            generation,
            chain: CallChain::current(),
            call: Box::new(TypedCall {
                reply,
                call: Some(call),
                value: None,
            }),
        };
        (job, answer)
    }
}

struct ActorInfo {
    plugin_id: String,
    capacity: usize,
    event_budget: Duration,
    callback_budget: Duration,
    started: Instant,
    next_full_warning_ms: AtomicU64,
    suppressed_full_warnings: AtomicU64,
    guest_warnings: SharedRateLimit,
}

impl ActorInfo {
    fn new(plugin_id: String, sandbox: &SandboxLimits) -> Self {
        Self {
            plugin_id,
            capacity: sandbox.queue_capacity,
            event_budget: sandbox.event_budget,
            callback_budget: sandbox.max_call_duration,
            started: Instant::now(),
            next_full_warning_ms: AtomicU64::new(0),
            suppressed_full_warnings: AtomicU64::new(0),
            guest_warnings: SharedRateLimit::new(GUEST_WARNING_INTERVAL, GUEST_WARNING_BURST),
        }
    }

    fn budget(&self, kind: CallKind) -> Duration {
        match kind {
            CallKind::Event => self.event_budget,
            CallKind::Callback => self.callback_budget,
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
    kind: CallKind,
    generation: Option<u64>,
}

impl InstanceRef {
    pub(crate) fn detached() -> Self {
        let (jobs, _) = mpsc::channel(1);
        Self {
            jobs: jobs.downgrade(),
            info: Arc::new(ActorInfo::new(String::new(), &SandboxLimits::default())),
            kind: CallKind::Callback,
            generation: None,
        }
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.info.plugin_id
    }

    pub(crate) fn admit_warning(&self) -> Option<u64> {
        self.info.guest_warnings.admit(Instant::now())
    }

    pub(crate) fn is_upstream(&self) -> bool {
        CallChain::current().contains(self.plugin_id())
    }

    pub(crate) fn for_calls(&self, kind: CallKind) -> Self {
        Self {
            kind,
            ..self.clone()
        }
    }

    pub(crate) fn stamped(&self, generation: u64) -> Self {
        Self {
            generation: Some(generation),
            ..self.clone()
        }
    }

    pub(crate) fn any_generation(&self) -> Self {
        Self {
            generation: None,
            ..self.clone()
        }
    }

    pub(crate) fn post<T, F>(&self, op: &'static str, call: F) -> Result<(), CallFailure>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        let answer = self.enqueue(op, call)?;
        tokio::spawn(async move {
            let _ = answer.await;
        });
        Ok(())
    }

    fn enqueue<T, F>(
        &self,
        op: &'static str,
        call: F,
    ) -> Result<oneshot::Receiver<Result<T, CallFailure>>, CallFailure>
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
        let deadline = Deadline::after(self.info.budget(self.kind));
        let (job, answer) = Job::new(op, JobKind::Call, Some(deadline), self.generation, call);
        match jobs.try_send(job) {
            Ok(()) => Ok(answer),
            Err(TrySendError::Full(_)) => {
                self.info.warn_queue_full(op);
                Err(CallFailure::QueueFull)
            }
            Err(TrySendError::Closed(_)) => Err(CallFailure::Stopped),
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
        let answer = self.enqueue(op, call)?;
        answer.await.unwrap_or(Err(CallFailure::Dropped))
    }
}

pub(crate) struct PluginActor {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    info: Arc<ActorInfo>,
    stopping: Arc<AtomicBool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl PluginActor {
    pub(crate) async fn start(factory: InstanceFactory) -> Result<Arc<Self>, WasmLoaderError> {
        let sandbox = *factory.sandbox();
        let (jobs, queue) = mpsc::channel(sandbox.queue_capacity);
        let info = Arc::new(ActorInfo::new(factory.plugin_id().to_owned(), &sandbox));
        let instance = InstanceRef {
            jobs: jobs.downgrade(),
            info: Arc::clone(&info),
            kind: CallKind::Callback,
            generation: None,
        };
        let supervisor = Supervisor::start(factory, instance).await?;
        let stopping = Arc::new(AtomicBool::new(false));
        let task =
            tokio::spawn(run(supervisor, queue, Arc::clone(&stopping)).with_current_subscriber());
        Ok(Arc::new(Self {
            jobs: Mutex::new(Some(jobs)),
            info,
            stopping,
            task: Mutex::new(Some(task)),
        }))
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.info.plugin_id
    }

    pub(crate) async fn call_lifecycle<T, F>(
        &self,
        op: &'static str,
        kind: JobKind,
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
        let (job, answer) = Job::new(op, kind, None, None, call);
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
    mut supervisor: Supervisor,
    mut queue: mpsc::Receiver<Job>,
    stopping: Arc<AtomicBool>,
) {
    loop {
        let job = match supervisor.retry_at() {
            Some(at) => tokio::select! {
                biased;
                job = queue.recv() => job,
                () = tokio::time::sleep_until(at) => {
                    supervisor.retry().await;
                    continue;
                }
            },
            None => queue.recv().await,
        };
        let Some(job) = job else {
            break;
        };
        if stopping.load(Ordering::Acquire) {
            job.call.refuse(CallFailure::Stopped);
            break;
        }
        if let ControlFlow::Break(()) = supervisor.handle(job).await {
            break;
        }
    }
    supervisor.retire();
}
