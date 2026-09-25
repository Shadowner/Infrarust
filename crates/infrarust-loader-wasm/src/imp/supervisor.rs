use std::any::Any;
use std::fmt;
use std::future::Future;
use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures_util::FutureExt;
use tokio::time::Instant;

use crate::actor::{CallFailure, GuestCall, InstanceRef, Job, JobKind};
use crate::deadline::Deadline;
use crate::error::WasmLoaderError;
use crate::instance::{InstanceFactory, LiveInstance};
use crate::recovery::{RestartBudget, Verdict};

const FIRST_GENERATION: u64 = 1;

enum Health {
    Starting(LiveInstance),
    Healthy(LiveInstance),
    Recovering,
    Quarantined { until: Instant },
    Failed,
}

enum Fault {
    Trapped(wasmtime::Error),
    Panicked(String),
    Overran(Duration),
    Refused(String),
    Unavailable(WasmLoaderError),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trapped(trap) => write!(f, "the guest trapped: {}", trap.root_cause()),
            Self::Panicked(message) => write!(f, "a host function panicked: {message}"),
            Self::Overran(limit) => write!(f, "the call ran past max_call_duration ({limit:?})"),
            Self::Refused(message) => write!(f, "on_enable returned an error: {message}"),
            Self::Unavailable(error) => write!(f, "no fresh instance could be created: {error}"),
        }
    }
}

impl Fault {
    fn failure(&self) -> CallFailure {
        match self {
            Self::Trapped(trap) => CallFailure::Trapped(trap.to_string()),
            other => CallFailure::Abandoned(other.to_string()),
        }
    }
}

pub(crate) struct Supervisor {
    factory: InstanceFactory,
    instance: InstanceRef,
    generation: u64,
    health: Health,
    budget: RestartBudget,
}

impl Supervisor {
    pub(crate) async fn start(
        factory: InstanceFactory,
        instance: InstanceRef,
    ) -> Result<Self, WasmLoaderError> {
        let live = factory
            .instantiate(FIRST_GENERATION, instance.stamped(FIRST_GENERATION))
            .await?;
        let budget = RestartBudget::new(factory.sandbox().recovery);
        Ok(Self {
            factory,
            instance,
            generation: FIRST_GENERATION,
            health: Health::Starting(live),
            budget,
        })
    }

    pub(crate) fn retry_at(&self) -> Option<Instant> {
        match self.health {
            Health::Quarantined { until } => Some(until),
            _ => None,
        }
    }

    pub(crate) async fn handle(&mut self, job: Job) -> ControlFlow<()> {
        let Job {
            op,
            kind,
            deadline,
            generation,
            mut call,
        } = job;
        if kind == JobKind::Disable {
            self.disable(op, call).await;
            return ControlFlow::Break(());
        }
        if call.caller_gone() {
            tracing::debug!(plugin = %self.factory.plugin_id(), op,
                "skipping a queued wasm guest call: its caller stopped waiting");
            return ControlFlow::Continue(());
        }
        if self.retry_at().is_some_and(|at| Instant::now() >= at) {
            self.retry().await;
        }
        let plugin_id = self.factory.plugin_id();
        let limit = self.factory.sandbox().max_call_duration;
        let live = match &mut self.health {
            Health::Starting(live) | Health::Healthy(live) => live,
            Health::Recovering | Health::Quarantined { .. } => {
                call.refuse(CallFailure::Quarantined);
                return ControlFlow::Continue(());
            }
            Health::Failed => {
                call.refuse(CallFailure::Failed);
                return ControlFlow::Continue(());
            }
        };
        if generation.is_some_and(|generation| generation != self.generation) {
            tracing::debug!(plugin = %plugin_id, op,
                "skipping a queued wasm guest call: the instance it was meant for was replaced");
            call.refuse(CallFailure::Replaced);
            return ControlFlow::Continue(());
        }
        if deadline.is_some_and(|deadline| deadline.has_passed()) {
            tracing::warn!(plugin = %plugin_id, op,
                "skipping a queued wasm guest call: its deadline passed while it waited");
            call.refuse(CallFailure::Expired);
            return ControlFlow::Continue(());
        }
        match run_guest(live, call.as_mut(), deadline, limit).await {
            Ok(()) => {
                if kind == JobKind::Enable {
                    self.promote();
                }
                call.answer();
            }
            Err(fault) => {
                let failure = fault.failure();
                self.fail(op, &fault).await;
                call.refuse(failure);
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) async fn retry(&mut self) {
        self.health = Health::Recovering;
        self.budget.retry(Instant::now());
        if !self.restart().await {
            self.recover().await;
        }
    }

    pub(crate) fn retire(mut self) {
        let health = std::mem::replace(&mut self.health, Health::Failed);
        if let Health::Starting(mut live) | Health::Healthy(mut live) = health {
            live.release_host_resources();
        }
        self.factory.registrations().fail_holds(None);
        tracing::debug!(plugin = %self.factory.plugin_id(), "wasm plugin task stopped");
    }

    async fn disable(&mut self, op: &'static str, mut call: Box<dyn GuestCall>) {
        let limit = self.factory.sandbox().max_call_duration;
        let live = match &mut self.health {
            Health::Starting(live) | Health::Healthy(live) => live,
            Health::Recovering | Health::Quarantined { .. } => {
                call.refuse(CallFailure::Quarantined);
                return;
            }
            Health::Failed => {
                call.refuse(CallFailure::Failed);
                return;
            }
        };
        match run_guest(live, call.as_mut(), None, limit).await {
            Ok(()) => call.answer(),
            Err(fault) => {
                self.report(op, &fault);
                call.refuse(fault.failure());
            }
        }
    }

    fn promote(&mut self) {
        self.health = match std::mem::replace(&mut self.health, Health::Recovering) {
            Health::Starting(live) => Health::Healthy(live),
            other => other,
        };
    }

    async fn fail(&mut self, op: &'static str, fault: &Fault) {
        self.report(op, fault);
        let enabled = matches!(self.health, Health::Healthy(_));
        let health = std::mem::replace(&mut self.health, Health::Recovering);
        if let Health::Starting(live) | Health::Healthy(live) = health {
            self.discard(live);
        }
        if enabled {
            self.recover().await;
        } else {
            self.health = Health::Failed;
        }
    }

    async fn recover(&mut self) {
        loop {
            match self.budget.after_fault(Instant::now()) {
                Verdict::Restart => {
                    if self.restart().await {
                        return;
                    }
                }
                Verdict::Quarantine { until, backoff } => {
                    let policy = self.budget.policy();
                    tracing::warn!(plugin = %self.factory.plugin_id(),
                        restarts = self.budget.restarts_in_window(),
                        max_restarts = policy.max_restarts, window = ?policy.window,
                        retry_in = ?backoff,
                        "wasm plugin quarantined: it kept failing; retrying after the backoff");
                    self.health = Health::Quarantined { until };
                    return;
                }
            }
        }
    }

    async fn restart(&mut self) -> bool {
        self.generation += 1;
        let generation = self.generation;
        let instance = self.instance.stamped(generation);
        let mut live = match self.factory.instantiate(generation, instance).await {
            Ok(live) => live,
            Err(error) => {
                self.report("instantiate", &Fault::Unavailable(error));
                return false;
            }
        };
        let limit = self.factory.sandbox().max_call_duration;
        match enable(&mut live, limit).await {
            Ok(()) => {
                let ctx = self.factory.ctx();
                for name in self.factory.registrations().sweep(generation) {
                    ctx.command_manager().unregister(&name);
                }
                tracing::info!(plugin = %self.factory.plugin_id(), generation,
                    "wasm plugin recovered: a fresh instance is enabled");
                self.health = Health::Healthy(live);
                true
            }
            Err(fault) => {
                self.report("on-enable", &fault);
                self.discard(live);
                false
            }
        }
    }

    fn discard(&self, mut live: LiveInstance) {
        live.release_host_resources();
        drop(live);
        self.factory
            .registrations()
            .fail_holds(Some(self.generation));
    }

    fn report(&self, op: &'static str, fault: &Fault) {
        tracing::error!(plugin = %self.factory.plugin_id(), op, generation = self.generation,
            cause = %fault, "wasm plugin instance failed; discarding it");
        if let Fault::Trapped(trap) = fault {
            tracing::debug!(plugin = %self.factory.plugin_id(), op, generation = self.generation,
                trap = ?trap, "wasm trap details");
        }
    }
}

async fn run_guest(
    live: &mut LiveInstance,
    call: &mut dyn GuestCall,
    deadline: Option<Deadline>,
    limit: Duration,
) -> Result<(), Fault> {
    live.begin_call(deadline);
    let outcome = contain(limit, call.run(&mut live.store, &live.bindings)).await;
    live.end_call();
    outcome
}

async fn enable(live: &mut LiveInstance, limit: Duration) -> Result<(), Fault> {
    live.begin_call(None);
    let enabling = live
        .bindings
        .infrarust_plugin_guest()
        .call_on_enable(&mut live.store);
    let outcome = contain(limit, enabling).await;
    live.end_call();
    outcome?.map_err(Fault::Refused)
}

async fn contain<T>(
    limit: Duration,
    running: impl Future<Output = wasmtime::Result<T>>,
) -> Result<T, Fault> {
    match tokio::time::timeout(limit, AssertUnwindSafe(running).catch_unwind()).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(trap))) => Err(Fault::Trapped(trap)),
        Ok(Err(payload)) => Err(Fault::Panicked(panic_message(payload.as_ref()).to_owned())),
        Err(_) => Err(Fault::Overran(limit)),
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
