use std::sync::Arc;
use std::time::Instant;

use infrarust_plugin_common::guest_panic::{bounded_guest_panic, is_guest_panic_line};
use wasmtime::StoreLimits;
use wasmtime_wasi::Rng;

use crate::consts::{CODEC_LOG_BURST, CODEC_LOG_INTERVAL};
use crate::engine::store_limits;
use crate::rate_limit::SharedRateLimit;

const MAX_RANDOM_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
pub(crate) struct CodecLog {
    plugin_id: String,
    limit: SharedRateLimit,
}

impl CodecLog {
    pub(crate) fn new(plugin_id: String) -> Self {
        Self {
            plugin_id,
            limit: SharedRateLimit::new(CODEC_LOG_INTERVAL, CODEC_LOG_BURST),
        }
    }

    fn admit(&self) -> Option<u64> {
        self.limit.admit(Instant::now())
    }

    pub(crate) fn emit(&self, level: GuestLevel, message: &str) {
        let plugin = self.plugin_id.as_str();
        match level {
            GuestLevel::Trace => {
                if tracing::enabled!(tracing::Level::TRACE)
                    && let Some(suppressed) = self.admit()
                {
                    tracing::trace!(plugin = %plugin, suppressed, "{message}");
                }
            }
            GuestLevel::Debug => {
                if tracing::enabled!(tracing::Level::DEBUG)
                    && let Some(suppressed) = self.admit()
                {
                    tracing::debug!(plugin = %plugin, suppressed, "{message}");
                }
            }
            GuestLevel::Info => {
                if tracing::enabled!(tracing::Level::INFO)
                    && let Some(suppressed) = self.admit()
                {
                    tracing::info!(plugin = %plugin, suppressed, "{message}");
                }
            }
            GuestLevel::Warn => {
                if tracing::enabled!(tracing::Level::WARN)
                    && let Some(suppressed) = self.admit()
                {
                    tracing::warn!(plugin = %plugin, suppressed, "{message}");
                }
            }
            GuestLevel::Error => {
                if tracing::enabled!(tracing::Level::ERROR)
                    && let Some(suppressed) = self.admit()
                {
                    tracing::error!(plugin = %plugin, suppressed, "{message}");
                }
            }
        }
    }
}

pub(crate) struct CodecStoreState {
    limits: StoreLimits,
    log: Arc<CodecLog>,
    rng: Option<Box<dyn Rng + Send>>,
    pub(crate) ticks: u64,
    pub(crate) budget_ticks: u64,
    guest_panic: Option<String>,
}

impl CodecStoreState {
    pub(crate) fn new(memory_bytes: usize, log: Arc<CodecLog>, budget_ticks: u64) -> Self {
        Self {
            limits: store_limits(memory_bytes),
            log,
            rng: None,
            ticks: 0,
            budget_ticks,
            guest_panic: None,
        }
    }

    pub(crate) fn limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }

    pub(crate) fn log(&mut self, level: GuestLevel, message: String) {
        if level == GuestLevel::Error && is_guest_panic_line(&message) {
            self.log.emit(GuestLevel::Debug, &message);
            self.guest_panic = Some(bounded_guest_panic(message));
            return;
        }
        self.log.emit(level, &message);
    }

    pub(crate) fn begin_call(&mut self) {
        self.ticks = 0;
        self.guest_panic = None;
    }

    pub(crate) fn take_guest_panic(&mut self) -> Option<String> {
        self.guest_panic.take()
    }

    fn rng(&mut self) -> &mut (dyn Rng + Send) {
        self.rng
            .get_or_insert_with(wasmtime_wasi::thread_rng)
            .as_mut()
    }

    pub(crate) fn random_u64(&mut self) -> u64 {
        self.rng().next_u64()
    }

    pub(crate) fn random_bytes(&mut self, len: u64) -> wasmtime::Result<Vec<u8>> {
        if len > MAX_RANDOM_BYTES {
            wasmtime::bail!(
                "codec filter asked for {len} random bytes; at most {MAX_RANDOM_BYTES} per call"
            );
        }
        let mut bytes = vec![0; usize::try_from(len)?];
        self.rng().fill_bytes(&mut bytes);
        Ok(bytes)
    }
}
