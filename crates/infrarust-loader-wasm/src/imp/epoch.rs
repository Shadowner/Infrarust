use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::Engine;

use crate::error::WasmLoaderError;
use crate::sync::lock;

const MIN_TICK: Duration = Duration::from_millis(1);

pub(crate) struct TickedEngine {
    engine: Engine,
    tick: Duration,
    ticker: Mutex<Option<EpochTicker>>,
}

impl TickedEngine {
    pub(crate) const fn new(engine: Engine, tick: Duration) -> Self {
        Self {
            engine,
            tick,
            ticker: Mutex::new(None),
        }
    }

    pub(crate) fn ticking(&self) -> Result<&Engine, WasmLoaderError> {
        let mut ticker = lock(&self.ticker);
        if ticker.is_none() {
            let spawned = EpochTicker::spawn(&self.engine, self.tick)
                .map_err(|error| WasmLoaderError::Engine(wasmtime::Error::from(error)))?;
            *ticker = Some(spawned);
        }
        Ok(&self.engine)
    }

    #[cfg(test)]
    fn is_ticking(&self) -> bool {
        lock(&self.ticker).is_some()
    }
}

pub(crate) struct EpochTicker {
    _thread: JoinHandle<()>,
}

impl EpochTicker {
    pub(crate) fn spawn(engine: &Engine, tick: Duration) -> std::io::Result<Self> {
        let tick = tick.max(MIN_TICK);
        let engine = engine.weak();
        let handle = std::thread::Builder::new()
            .name("infrarust-wasm-epoch".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(tick);
                    match engine.upgrade() {
                        Some(engine) => engine.increment_epoch(),
                        None => break,
                    }
                }
            })?;
        Ok(Self { _thread: handle })
    }

    #[cfg(test)]
    fn into_handle(self) -> JoinHandle<()> {
        self._thread
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use wasmtime::{Config, Instance, Module, Store, Trap};

    const COUNTDOWN: &str = r#"
        (module
            (func (export "run") (param $left i32)
                (loop $again
                    (local.set $left (i32.sub (local.get $left) (i32.const 1)))
                    (br_if $again (i32.gt_u (local.get $left) (i32.const 0))))))
    "#;

    fn ticks_under(store: &mut Store<()>, module: &Module) -> bool {
        let instance = Instance::new(&mut *store, module, &[]).unwrap();
        let run = instance
            .get_typed_func::<i32, ()>(&mut *store, "run")
            .unwrap();
        store.set_epoch_deadline(3);
        match run.call(&mut *store, i32::MAX) {
            Ok(()) => false,
            Err(error) => error.downcast_ref::<Trap>() == Some(&Trap::Interrupt),
        }
    }

    #[test]
    fn the_clock_starts_with_the_first_use_of_the_engine() {
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = TickedEngine::new(Engine::new(&config).unwrap(), MIN_TICK);
        assert!(!engine.is_ticking());

        let module = Module::new(engine.ticking().unwrap(), COUNTDOWN).unwrap();
        assert!(engine.is_ticking());
        let mut store = Store::new(engine.ticking().unwrap(), ());
        assert!(ticks_under(&mut store, &module));
    }

    #[test]
    fn the_clock_keeps_ticking_while_the_engine_lives_and_stops_after() {
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = Engine::new(&config).unwrap();
        let module = Module::new(&engine, COUNTDOWN).unwrap();
        let mut store = Store::new(&engine, ());
        let handle = EpochTicker::spawn(&engine, MIN_TICK).unwrap().into_handle();
        drop(engine);
        assert!(
            ticks_under(&mut store, &module),
            "a store still running code keeps the clock ticking after the loader dropped its engine"
        );
        drop(module);
        drop(store);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "the ticker outlived its engine"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        handle.join().unwrap();
    }
}
