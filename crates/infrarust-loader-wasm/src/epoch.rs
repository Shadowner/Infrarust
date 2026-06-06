use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;

use wasmtime::Engine;

use crate::consts::EPOCH_TICK_INTERVAL;

pub(crate) struct EpochTicker {
    stop_tx: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    pub(crate) fn spawn(engine: Engine) -> Self {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("infrarust-wasm-epoch".to_owned())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stop_rx.recv_timeout(EPOCH_TICK_INTERVAL)
                {
                    engine.increment_epoch();
                }
            })
            .expect("spawning the wasm epoch ticker thread should not fail");
        Self {
            stop_tx: Some(stop_tx),
            handle: Some(handle),
        }
    }

    fn stop(&mut self) {
        drop(self.stop_tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop();
    }
}
