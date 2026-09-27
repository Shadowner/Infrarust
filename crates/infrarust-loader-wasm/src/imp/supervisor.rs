use std::any::Any;
use std::fmt;
use std::future::Future;
use std::ops::ControlFlow;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use tokio::time::Instant;

use crate::actor::{CallFailure, CallKind, GuestCall, Halt, InstanceRef, Job, JobKind};
use crate::bindings::exports::infrarust::plugin::guest::{EnableReason, RecoveryInfo};
use crate::chain::CallChain;
use crate::consts::{FAR_FUTURE, GUEST_WARNING_BURST, GUEST_WARNING_INTERVAL};
use crate::deadline::Deadline;
use crate::error::WasmLoaderError;
use crate::instance::{InstanceFactory, LiveInstance};
use crate::rate_limit::RateLimit;
use crate::recovery::{RestartBudget, Verdict};

const FIRST_GENERATION: u64 = 1;

enum Health {
    Starting(LiveInstance),
    Healthy(LiveInstance),
    Recovering,
    Quarantined { until: Instant },
    Failed,
    Halted,
}

#[derive(Debug)]
pub(crate) struct EnableRefused(pub(crate) String);

impl fmt::Display for EnableRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "on_enable returned an error: {}", self.0)
    }
}

impl std::error::Error for EnableRefused {}

#[derive(Debug)]
pub(crate) enum Fault {
    Trapped(Arc<wasmtime::Error>),
    Panicked(String),
    Overran(Duration),
    PastDeadline(CallKind),
    Refused(String),
    Unavailable(WasmLoaderError),
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Trapped(trap) => write!(f, "the guest trapped: {}", trap.root_cause()),
            Self::Panicked(message) => write!(f, "a host function panicked: {message}"),
            Self::Overran(limit) => write!(f, "the call ran past max_call_duration ({limit:?})"),
            Self::PastDeadline(CallKind::Event) => {
                f.write_str("the call ran past the event deadline")
            }
            Self::PastDeadline(CallKind::Callback) => f.write_str(
                "the call ran past its deadline (max_call_duration after it was queued)",
            ),
            Self::Refused(message) => write!(f, "on_enable returned an error: {message}"),
            Self::Unavailable(error) => write!(f, "no fresh instance could be created: {error}"),
        }
    }
}

impl Fault {
    fn failure(self: &Arc<Self>) -> CallFailure {
        match &**self {
            Self::Trapped(trap) => CallFailure::Trapped(Arc::clone(trap)),
            _ => CallFailure::Abandoned(Arc::clone(self)),
        }
    }

    pub(crate) fn refusal(&self) -> Option<&str> {
        match self {
            Self::Refused(message) => Some(message),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Bound {
    at: Instant,
    limit: Duration,
    deadline: Option<CallKind>,
}

impl Bound {
    fn new(limit: Duration, deadline: Option<(Deadline, CallKind)>) -> Self {
        let now = Instant::now();
        let own = now.checked_add(limit).unwrap_or(now + FAR_FUTURE);
        match deadline {
            Some((deadline, kind)) if deadline.expires() < own => Self {
                at: deadline.expires(),
                limit,
                deadline: Some(kind),
            },
            _ => Self {
                at: own,
                limit,
                deadline: None,
            },
        }
    }

    const fn fault(self) -> Fault {
        match self.deadline {
            Some(kind) => Fault::PastDeadline(kind),
            None => Fault::Overran(self.limit),
        }
    }
}

pub(crate) struct Supervisor {
    factory: InstanceFactory,
    instance: InstanceRef,
    generation: u64,
    health: Health,
    budget: RestartBudget,
    last_fault: String,
    halt: Arc<Halt>,
    expired_warnings: RateLimit,
}

impl Supervisor {
    pub(crate) async fn start(
        factory: InstanceFactory,
        instance: InstanceRef,
        halt: Arc<Halt>,
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
            last_fault: String::new(),
            halt,
            expired_warnings: RateLimit::new(GUEST_WARNING_INTERVAL, GUEST_WARNING_BURST),
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
            chain,
            mut call,
        } = job;
        let chain = chain.with(self.factory.plugin_id());
        if kind == JobKind::Disable {
            self.disable(op, call, chain).await;
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
            Health::Halted => {
                call.refuse(CallFailure::Stopped);
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
            if let Some(suppressed) = self.expired_warnings.admit(std::time::Instant::now()) {
                tracing::warn!(plugin = %plugin_id, op, suppressed,
                    "skipping a queued wasm guest call: its deadline passed while it waited");
            }
            call.refuse(CallFailure::Expired);
            return ControlFlow::Continue(());
        }
        let call_kind = match kind {
            JobKind::Call(call_kind) => Some(call_kind),
            JobKind::Enable | JobKind::Disable => None,
        };
        let bound = Bound::new(limit, deadline.zip(call_kind));
        match run_guest(live, call.as_mut(), deadline, bound, chain.clone()).await {
            Ok(()) => {
                if kind == JobKind::Enable {
                    self.promote();
                }
                call.answer();
            }
            Err(fault) => {
                let fault = Arc::new(fault);
                let failure = fault.failure();
                self.fail(op, &fault, &chain).await;
                call.refuse(failure);
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) async fn retry(&mut self) {
        if self.halted() {
            return;
        }
        self.health = Health::Recovering;
        self.budget.retry(Instant::now());
        let chain = CallChain::default().with(self.factory.plugin_id());
        if !self.restart(&chain).await {
            self.recover(&chain).await;
        }
    }

    fn halted(&mut self) -> bool {
        if !self.halt.requested() {
            return false;
        }
        tracing::info!(plugin = %self.factory.plugin_id(),
            "wasm plugin recovery stopped: the plugin is being disabled or unloaded");
        self.health = Health::Halted;
        true
    }

    pub(crate) fn retire(mut self) {
        let health = std::mem::replace(&mut self.health, Health::Failed);
        if let Health::Starting(mut live) | Health::Healthy(mut live) = health {
            live.release_host_resources();
        }
        self.factory.registrations().fail_holds(None);
        self.withdraw_codec_filters();
        tracing::debug!(plugin = %self.factory.plugin_id(), "wasm plugin task stopped");
    }

    fn withdraw_codec_filters(&self) {
        let filters = self.factory.registrations().take_codec_filters();
        if filters.is_empty() {
            return;
        }
        let ctx = self.factory.ctx();
        let Some(registry) = ctx.codec_filters() else {
            return;
        };
        for id in filters {
            if let Err(e) = registry.unregister(&id) {
                tracing::debug!(plugin = %self.factory.plugin_id(), filter = %id,
                    "codec filter already gone when the plugin stopped: {e}");
            }
        }
    }

    async fn disable(&mut self, op: &'static str, mut call: Box<dyn GuestCall>, chain: CallChain) {
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
            Health::Halted => {
                call.refuse(CallFailure::Stopped);
                return;
            }
        };
        match run_guest(live, call.as_mut(), None, Bound::new(limit, None), chain).await {
            Ok(()) => call.answer(),
            Err(fault) => {
                let fault = Arc::new(fault);
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

    async fn fail(&mut self, op: &'static str, fault: &Fault, chain: &CallChain) {
        self.report(op, fault);
        self.last_fault = fault.to_string();
        let enabled = matches!(self.health, Health::Healthy(_));
        let health = std::mem::replace(&mut self.health, Health::Recovering);
        if let Health::Starting(live) | Health::Healthy(live) = health {
            self.discard(live);
        }
        if enabled {
            self.recover(chain).await;
        } else {
            self.health = Health::Failed;
        }
    }

    async fn recover(&mut self, chain: &CallChain) {
        loop {
            if self.halted() {
                return;
            }
            match self.budget.after_fault(Instant::now()) {
                Verdict::Restart => {
                    if self.restart(chain).await {
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

    async fn restart(&mut self, chain: &CallChain) -> bool {
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
        let reason = EnableReason::Recovered(RecoveryInfo {
            attempt: u32::try_from(generation - FIRST_GENERATION).unwrap_or(u32::MAX),
            cause: self.last_fault.clone(),
        });
        match chain.clone().scope(enable(&mut live, limit, &reason)).await {
            Ok(()) => {
                let ctx = self.factory.ctx();
                let stale = self.factory.registrations().sweep(generation);
                for name in stale.commands {
                    if let Err(e) = ctx.command_manager().unregister(&name) {
                        tracing::debug!(plugin = %self.factory.plugin_id(), "stale command already gone: {e}");
                    }
                }
                for registration in stale.limbo {
                    if !registration.unregister() {
                        tracing::debug!(plugin = %self.factory.plugin_id(), name = registration.name(),
                            "stale limbo handler already gone");
                    }
                }
                tracing::info!(plugin = %self.factory.plugin_id(), generation,
                    "wasm plugin recovered: a fresh instance is enabled");
                self.health = Health::Healthy(live);
                true
            }
            Err(fault) => {
                self.report("on-enable", &fault);
                self.last_fault = fault.to_string();
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
    bound: Bound,
    chain: CallChain,
) -> Result<(), Fault> {
    live.begin_call(deadline);
    let running = call.run(&mut live.store, &live.bindings);
    let outcome = chain.scope(contain(bound, running)).await;
    live.end_call();
    outcome
}

async fn enable(
    live: &mut LiveInstance,
    limit: Duration,
    reason: &EnableReason,
) -> Result<(), Fault> {
    live.begin_call(None);
    let enabling = live
        .bindings
        .infrarust_plugin_guest()
        .call_on_enable(&mut live.store, reason);
    let outcome = contain(Bound::new(limit, None), enabling).await;
    live.end_call();
    outcome?.map_err(Fault::Refused)
}

async fn contain<T>(
    bound: Bound,
    running: impl Future<Output = wasmtime::Result<T>>,
) -> Result<T, Fault> {
    match tokio::time::timeout_at(bound.at, AssertUnwindSafe(running).catch_unwind()).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => Err(match error.downcast::<EnableRefused>() {
            Ok(refused) => Fault::Refused(refused.0),
            Err(trap) => Fault::Trapped(Arc::new(trap)),
        }),
        Ok(Err(payload)) => Err(Fault::Panicked(panic_message(payload.as_ref()).to_owned())),
        Err(_) => Err(bound.fault()),
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
