pub(crate) mod bindings;
mod convert;
mod host;
mod linker;
mod proxies;
mod quarantine;
mod store_state;

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::filter::CodecSessionInit;
use infrarust_config::WasmCodecQuarantineConfig;
use wasmtime::component::{Component, InstancePre, ResourceAny};
use wasmtime::{Engine, Store, Trap, UpdateDeadline};

pub(crate) use proxies::WasmCodecFilterFactory;

use bindings::exports::infrarust::plugin::codec_filter::{
    FilterOutput, FilterVerdict, Guest as CodecGuest, GuestIndices,
};
use linker::build_codec_linker;
use store_state::{CodecLog, CodecStoreState};

use crate::bindings::infrarust::plugin::types::ConnectionState as WitConnectionState;
use crate::config::SandboxLimits;
use crate::error::WasmLoaderError;

pub(crate) struct CodecInstantiator {
    engine: Engine,
    pre: InstancePre<CodecStoreState>,
    indices: GuestIndices,
    plugin_id: String,
    log: Arc<CodecLog>,
    memory_bytes: usize,
    budget_ticks: u64,
    budget: Duration,
    quarantine: WasmCodecQuarantineConfig,
}

impl CodecInstantiator {
    pub(crate) fn new(
        engine: Engine,
        component: &Component,
        plugin_id: String,
        sandbox: &SandboxLimits,
    ) -> Result<Self, WasmLoaderError> {
        let linker = build_codec_linker(&engine, component, &plugin_id)?;
        let pre = linker
            .instantiate_pre(component)
            .map_err(|e| instantiate_err(&plugin_id, "instantiate_pre", &e))?;
        let indices = GuestIndices::new(&pre)
            .map_err(|e| instantiate_err(&plugin_id, "guest-indices", &e))?;
        Ok(Self {
            engine,
            pre,
            indices,
            log: Arc::new(CodecLog::new(plugin_id.clone())),
            plugin_id,
            memory_bytes: sandbox.memory_bytes,
            budget_ticks: sandbox.codec_deadline_ticks,
            budget: sandbox.codec_cpu_budget,
            quarantine: sandbox.codec_quarantine,
        })
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub(crate) const fn budget(&self) -> Duration {
        self.budget
    }

    pub(crate) const fn quarantine(&self) -> WasmCodecQuarantineConfig {
        self.quarantine
    }

    fn guest_failure(
        &self,
        store: &mut Store<CodecStoreState>,
        error: &wasmtime::Error,
    ) -> CreateFailure {
        CreateFailure::Guest(cause(
            error,
            self.budget,
            store.data_mut().take_guest_panic(),
        ))
    }

    fn create_live(&self, factory_id: u64, init: &CodecSessionInit) -> Result<Live, CreateFailure> {
        let mut store = Store::new(
            &self.engine,
            CodecStoreState::new(self.memory_bytes, Arc::clone(&self.log), self.budget_ticks),
        );
        store.limiter(|s: &mut CodecStoreState| {
            s.limits_mut() as &mut dyn wasmtime::ResourceLimiter
        });
        install_budget(&mut store);
        arm(&mut store);
        let instance = match self.pre.instantiate(&mut store) {
            Ok(instance) => instance,
            Err(error) if error.downcast_ref::<Trap>().is_some() => {
                return Err(self.guest_failure(&mut store, &error));
            }
            Err(error) => {
                return Err(CreateFailure::Host(instantiate_err(
                    &self.plugin_id,
                    "instantiate",
                    &error,
                )));
            }
        };
        let guest = self
            .indices
            .load(&mut store, &instance)
            .map_err(|e| CreateFailure::Host(instantiate_err(&self.plugin_id, "guest-load", &e)))?;
        let wit_init = convert::session_init_to_wit(init);
        let handle = guest
            .call_create(&mut store, factory_id, wit_init)
            .map_err(|error| self.guest_failure(&mut store, &error))?;
        Ok(Live {
            store,
            guest,
            handle,
        })
    }
}

pub(crate) enum CreateFailure {
    Host(WasmLoaderError),
    Guest(String),
}

pub(crate) fn install_budget(store: &mut Store<CodecStoreState>) {
    store.epoch_deadline_callback(|mut ctx| {
        let state = ctx.data_mut();
        state.ticks = state.ticks.saturating_add(1);
        if state.ticks > state.budget_ticks {
            Ok(UpdateDeadline::Interrupt)
        } else {
            Ok(UpdateDeadline::Continue(1))
        }
    });
}

pub(crate) fn arm(store: &mut Store<CodecStoreState>) {
    store.data_mut().begin_call();
    store.set_epoch_deadline(1);
}

pub(crate) fn cause(error: &wasmtime::Error, budget: Duration, panic: Option<String>) -> String {
    if let Some(panic) = panic {
        return panic;
    }
    match error.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => format!("ran past codec_cpu_budget ({budget:?})"),
        Some(Trap::UnreachableCodeReached) => "hit an unreachable instruction".to_owned(),
        Some(Trap::StackOverflow) => "overflowed its stack".to_owned(),
        Some(trap) => format!("trapped: {trap}"),
        None => format!("failed: {}", error.root_cause()),
    }
}

pub(crate) enum Outcome {
    Pass,
    Drop,
    Output(FilterOutput),
}

pub(crate) struct Live {
    store: Store<CodecStoreState>,
    guest: CodecGuest,
    handle: ResourceAny,
}

impl Live {
    pub(crate) fn filter(&mut self, packet_id: i32, data: &[u8]) -> wasmtime::Result<Outcome> {
        arm(&mut self.store);
        let guest = self.guest.filter_instance();
        match guest.call_filter(&mut self.store, self.handle, packet_id, data)? {
            FilterVerdict::Pass => Ok(Outcome::Pass),
            FilterVerdict::Drop => Ok(Outcome::Drop),
            FilterVerdict::Modified => Ok(Outcome::Output(
                guest.call_take_output(&mut self.store, self.handle)?,
            )),
        }
    }

    pub(crate) fn on_state_change(&mut self, state: WitConnectionState) -> wasmtime::Result<()> {
        arm(&mut self.store);
        self.guest
            .filter_instance()
            .call_on_state_change(&mut self.store, self.handle, state)
    }

    pub(crate) fn on_compression_change(&mut self, threshold: i32) -> wasmtime::Result<()> {
        arm(&mut self.store);
        self.guest.filter_instance().call_on_compression_change(
            &mut self.store,
            self.handle,
            threshold,
        )
    }

    pub(crate) fn on_encryption_enabled(&mut self) -> wasmtime::Result<()> {
        arm(&mut self.store);
        self.guest
            .filter_instance()
            .call_on_encryption_enabled(&mut self.store, self.handle)
    }

    pub(crate) fn on_close(&mut self) -> wasmtime::Result<()> {
        arm(&mut self.store);
        self.guest
            .filter_instance()
            .call_on_close(&mut self.store, self.handle)
    }

    pub(crate) fn take_guest_panic(&mut self) -> Option<String> {
        self.store.data_mut().take_guest_panic()
    }

    pub(crate) fn release(mut self) {
        arm(&mut self.store);
        let _ = self.handle.resource_drop(&mut self.store);
    }
}

fn instantiate_err(plugin_id: &str, what: &str, e: &wasmtime::Error) -> WasmLoaderError {
    WasmLoaderError::Instantiate {
        plugin_id: plugin_id.to_owned(),
        reason: format!("codec {what}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use wasmtime::{Config, Linker, Module};

    const SPIN: &str = r#"
        (module
            (import "host" "preempt" (func $preempt))
            (func (export "run") (param $preempted i32) (param $spins i32)
                (if (local.get $preempted) (then (call $preempt)))
                (loop $again
                    (local.set $spins (i32.sub (local.get $spins) (i32.const 1)))
                    (br_if $again (i32.gt_s (local.get $spins) (i32.const 0))))))
    "#;

    fn engine() -> Engine {
        let mut config = Config::new();
        config.epoch_interruption(true);
        Engine::new(&config).unwrap()
    }

    fn store(engine: &Engine, budget_ticks: u64) -> Store<CodecStoreState> {
        let mut store = Store::new(
            engine,
            CodecStoreState::new(
                1 << 20,
                Arc::new(CodecLog::new("budget-test".to_owned())),
                budget_ticks,
            ),
        );
        install_budget(&mut store);
        arm(&mut store);
        store
    }

    fn run(
        engine: &Engine,
        store: &mut Store<CodecStoreState>,
        preempted: bool,
    ) -> wasmtime::Result<u64> {
        let mut linker = Linker::new(engine);
        let ticker = engine.clone();
        linker.func_wrap("host", "preempt", move || {
            for _ in 0..50 {
                ticker.increment_epoch();
            }
        })?;
        let module = Module::new(engine, SPIN)?;
        let instance = linker.instantiate(&mut *store, &module)?;
        let run = instance.get_typed_func::<(i32, i32), ()>(&mut *store, "run")?;
        arm(store);
        run.call(&mut *store, (i32::from(preempted), 1_000))?;
        Ok(store.data().ticks)
    }

    #[test]
    fn a_call_preempted_for_many_ticks_counts_one_tick_of_its_budget() {
        let engine = engine();
        let mut store = store(&engine, 5);
        let ticks = run(&engine, &mut store, true).expect("a preempted call keeps its budget");
        assert_eq!(ticks, 1);
    }

    #[test]
    fn a_call_after_idle_ticks_starts_with_its_whole_budget() {
        let engine = engine();
        let mut store = store(&engine, 5);
        for _ in 0..10 {
            engine.increment_epoch();
        }
        assert_eq!(run(&engine, &mut store, false).unwrap(), 0);
    }

    #[test]
    fn a_call_that_keeps_running_past_its_budget_is_interrupted() {
        let engine = engine();
        let mut store = store(&engine, 2);
        let mut linker = Linker::new(&engine);
        let ticker = engine.clone();
        linker
            .func_wrap("host", "preempt", move || ticker.increment_epoch())
            .unwrap();
        let module = Module::new(
            &engine,
            r#"
            (module
                (import "host" "preempt" (func $tick))
                (func (export "spin") (loop $forever (call $tick) (br $forever))))
            "#,
        )
        .unwrap();
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .unwrap();
        arm(&mut store);
        let error = spin.call(&mut store, ()).unwrap_err();
        assert_eq!(error.downcast_ref::<Trap>(), Some(&Trap::Interrupt));
        assert_eq!(store.data().ticks, 3);
        assert_eq!(
            cause(&error, Duration::from_millis(5), None),
            "ran past codec_cpu_budget (5ms)"
        );
    }
}
