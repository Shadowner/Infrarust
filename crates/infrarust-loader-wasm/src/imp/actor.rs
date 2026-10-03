use std::fmt;
use std::future::Future;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use infrarust_api::__private::caller_deadline;
use infrarust_api::event::BoxFuture;
use infrarust_api::plugin::PluginRuntimeStatus;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;
use wasmtime::Store;

use crate::bindings::Plugin as PluginBindings;
use crate::chain::CallChain;
use crate::config::SandboxLimits;
use crate::consts::{GUEST_WARNING_BURST, GUEST_WARNING_INTERVAL, QUEUE_FULL_WARN_INTERVAL};
use crate::deadline::{Deadline, inside};
use crate::error::WasmLoaderError;
use crate::events::AccessListeners;
use crate::instance::InstanceFactory;
use crate::rate_limit::SharedRateLimit;
use crate::snapshots::PermissionSnapshots;
use crate::status::StatusBoard;
use crate::store_state::PluginStoreState;
use crate::supervisor::Fault;
use crate::supervisor::Supervisor;

#[derive(Debug, Clone)]
pub(crate) enum CallFailure {
    Stopped,
    QueueFull,
    Failed,
    Quarantined,
    Replaced,
    Expired,
    TimedOut,
    Trapped(Arc<wasmtime::Error>),
    Abandoned(Arc<Fault>),
    Dropped,
}

impl PartialEq for CallFailure {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Trapped(a), Self::Trapped(b)) => Arc::ptr_eq(a, b),
            (Self::Abandoned(a), Self::Abandoned(b)) => Arc::ptr_eq(a, b),
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

impl Eq for CallFailure {}

impl std::error::Error for CallFailure {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallKind {
    Event,
    Callback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobKind {
    Call(CallKind),
    Enable,
    Disable,
}

#[derive(Debug, Default)]
pub(crate) struct Halt {
    stopping: AtomicBool,
    disabling: AtomicBool,
    forced: AtomicBool,
    cut: Notify,
    proxy: CancellationToken,
}

impl Halt {
    pub(crate) fn new(proxy: CancellationToken) -> Self {
        Self {
            proxy,
            ..Self::default()
        }
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        if self.proxy_stopping() {
            self.forced.store(true, Ordering::Release);
            self.cut.notify_waiters();
        }
    }

    fn disable(&self) {
        self.disabling.store(true, Ordering::Release);
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(crate) fn proxy_stopping(&self) -> bool {
        self.proxy.is_cancelled()
    }

    pub(crate) fn requested(&self) -> bool {
        self.stopping() || self.disabling.load(Ordering::Acquire) || self.proxy_stopping()
    }

    pub(crate) fn interrupted(&self) -> impl Future<Output = ()> + Send + '_ {
        let started_before_shutdown = !self.proxy_stopping();
        async move {
            let cut = self.cut.notified();
            if self.forced.load(Ordering::Acquire) {
                return;
            }
            if started_before_shutdown {
                tokio::select! {
                    () = cut => {}
                    () = self.proxy.cancelled() => {}
                }
            } else {
                cut.await;
            }
        }
    }
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
            Self::TimedOut => f.write_str("the plugin did not answer before the call's deadline"),
            Self::Trapped(trap) => write!(f, "the guest trapped: {}", trap.root_cause()),
            Self::Abandoned(fault) => write!(f, "{fault}"),
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

type Reply<T> = Option<oneshot::Sender<Result<T, CallFailure>>>;

struct TypedCall<T, F> {
    reply: Reply<T>,
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
        self.reply.as_ref().is_some_and(oneshot::Sender::is_closed)
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
        if let Some(reply) = reply {
            let _ = reply.send(value.ok_or(CallFailure::Dropped));
        }
    }

    fn refuse(self: Box<Self>, failure: CallFailure) {
        if let Some(reply) = self.reply {
            let _ = reply.send(Err(failure));
        }
    }
}

pub(crate) struct Job {
    pub(crate) op: &'static str,
    pub(crate) kind: JobKind,
    pub(crate) deadline: Option<Deadline>,
    pub(crate) generation: Option<u64>,
    pub(crate) chain: CallChain,
    pub(crate) call: Box<dyn GuestCall>,
    pub(crate) queued: Instant,
}

impl Job {
    fn new<T, F>(
        op: &'static str,
        kind: JobKind,
        deadline: Option<Deadline>,
        generation: Option<u64>,
        chain: CallChain,
        reply: Reply<T>,
        call: F,
    ) -> Self
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        Self {
            op,
            kind,
            deadline,
            generation,
            chain,
            call: Box::new(TypedCall {
                reply,
                call: Some(call),
                value: None,
            }),
            queued: Instant::now(),
        }
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
    loop_warnings: SharedRateLimit,
    snapshots: Arc<PermissionSnapshots>,
    access: Arc<AccessListeners>,
    board: StatusBoard,
}

impl ActorInfo {
    fn new(
        plugin_id: String,
        sandbox: &SandboxLimits,
        snapshots: Arc<PermissionSnapshots>,
    ) -> Self {
        Self {
            plugin_id,
            capacity: sandbox.queue_capacity,
            event_budget: inside(sandbox.event_budget),
            callback_budget: sandbox.max_call_duration,
            started: Instant::now(),
            next_full_warning_ms: AtomicU64::new(0),
            suppressed_full_warnings: AtomicU64::new(0),
            guest_warnings: SharedRateLimit::new(GUEST_WARNING_INTERVAL, GUEST_WARNING_BURST),
            loop_warnings: SharedRateLimit::new(GUEST_WARNING_INTERVAL, GUEST_WARNING_BURST),
            snapshots,
            access: Arc::default(),
            board: StatusBoard::new(sandbox.queue_capacity, sandbox.recovery),
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
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        let (jobs, _) = mpsc::channel(1);
        Self {
            jobs: jobs.downgrade(),
            info: Arc::new(ActorInfo::new(
                String::new(),
                &SandboxLimits::default(),
                Arc::default(),
            )),
            kind: CallKind::Callback,
            generation: None,
        }
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.info.plugin_id
    }

    pub(crate) fn snapshots(&self) -> &Arc<PermissionSnapshots> {
        &self.info.snapshots
    }

    pub(crate) fn access(&self) -> &Arc<AccessListeners> {
        &self.info.access
    }

    pub(crate) fn board(&self) -> &StatusBoard {
        &self.info.board
    }

    pub(crate) fn admit_warning(&self) -> Option<u64> {
        self.info.guest_warnings.admit(Instant::now())
    }

    pub(crate) fn admit_loop_warning(&self) -> Option<u64> {
        self.info.loop_warnings.admit(Instant::now())
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
        self.enqueue(op, CallChain::current().unawaited(), None, call)
            .map(drop)
    }

    fn awaited<T, F>(
        &self,
        op: &'static str,
        call: F,
    ) -> Result<(oneshot::Receiver<Result<T, CallFailure>>, Deadline), CallFailure>
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
        let deadline = self.enqueue(op, CallChain::current(), Some(reply), call)?;
        Ok((answer, deadline))
    }

    fn enqueue<T, F>(
        &self,
        op: &'static str,
        chain: CallChain,
        reply: Reply<T>,
        call: F,
    ) -> Result<Deadline, CallFailure>
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
        let deadline = Deadline::within(self.info.budget(self.kind), caller_deadline::current());
        let job = Job::new(
            op,
            JobKind::Call(self.kind),
            Some(deadline),
            self.generation,
            chain,
            reply,
            call,
        );
        match jobs.try_send(job) {
            Ok(()) => Ok(deadline),
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
        let (answer, deadline) = self.awaited(op, call)?;
        match tokio::time::timeout_at(deadline.expires(), answer).await {
            Ok(answer) => answer.unwrap_or(Err(CallFailure::Dropped)),
            Err(_) => Err(CallFailure::TimedOut),
        }
    }

    pub(crate) async fn call_or_none<T, F>(&self, op: &'static str, call: F) -> Option<T>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
                &'a mut Store<PluginStoreState>,
                &'a PluginBindings,
            ) -> BoxFuture<'a, wasmtime::Result<T>>
            + Send
            + 'static,
    {
        self.call(op, call).await.ok()
    }
}

pub(crate) struct PluginActor {
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    info: Arc<ActorInfo>,
    halt: Arc<Halt>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl PluginActor {
    pub(crate) async fn start(factory: InstanceFactory) -> Result<Arc<Self>, WasmLoaderError> {
        let sandbox = *factory.sandbox();
        let (jobs, queue) = mpsc::channel(sandbox.queue_capacity);
        let info = Arc::new(ActorInfo::new(
            factory.plugin_id().to_owned(),
            &sandbox,
            Arc::clone(factory.registrations().snapshots()),
        ));
        let instance = InstanceRef {
            jobs: jobs.downgrade(),
            info: Arc::clone(&info),
            kind: CallKind::Callback,
            generation: None,
        };
        let halt = Arc::new(Halt::new(factory.ctx().proxy_shutdown()));
        let supervisor = Supervisor::start(factory, instance, Arc::clone(&halt)).await?;
        let task =
            tokio::spawn(run(supervisor, queue, Arc::clone(&halt)).with_current_subscriber());
        Ok(Arc::new(Self {
            jobs: Mutex::new(Some(jobs)),
            info,
            halt,
            task: Mutex::new(Some(task)),
        }))
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.info.plugin_id
    }

    pub(crate) fn runtime_status(&self) -> PluginRuntimeStatus {
        let depth = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |jobs| {
                jobs.max_capacity().saturating_sub(jobs.capacity())
            });
        self.info.board.snapshot(depth)
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
        if kind == JobKind::Disable {
            self.halt.disable();
        }
        let (reply, answer) = oneshot::channel();
        let job = Job::new(
            op,
            kind,
            None,
            None,
            CallChain::current(),
            Some(reply),
            call,
        );
        if jobs.send(job).await.is_err() {
            return Err(CallFailure::Stopped);
        }
        drop(jobs);
        answer.await.unwrap_or(Err(CallFailure::Dropped))
    }

    pub(crate) fn stop(&self) {
        self.halt.stop();
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

async fn run(mut supervisor: Supervisor, mut queue: mpsc::Receiver<Job>, halt: Arc<Halt>) {
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
        supervisor.taken(&job, queue.len());
        if halt.stopping() {
            job.call.refuse(CallFailure::Stopped);
            break;
        }
        if let ControlFlow::Break(()) = supervisor.handle(job).await {
            break;
        }
    }
    supervisor.retire();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn job(reply: Reply<()>) -> Job {
        Job::new(
            "op",
            JobKind::Call(CallKind::Event),
            None,
            None,
            CallChain::default(),
            reply,
            |_: &mut Store<PluginStoreState>, _: &PluginBindings| Box::pin(async { Ok(()) }),
        )
    }

    #[test]
    fn a_posted_call_is_never_skipped_for_want_of_a_waiting_caller() {
        assert!(!job(None).call.caller_gone());
    }

    #[test]
    fn an_awaited_call_is_skipped_once_its_caller_stops_waiting() {
        let (reply, answer) = oneshot::channel();
        let job = job(Some(reply));
        assert!(!job.call.caller_gone());
        drop(answer);
        assert!(job.call.caller_gone());
    }
}
