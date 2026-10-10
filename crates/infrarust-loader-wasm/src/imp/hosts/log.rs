use infrarust_plugin_common::guest_panic::is_guest_panic_line;
use tracing::Level;
use tracing::level_filters::{LevelFilter, STATIC_MAX_LEVEL};

use crate::bindings::infrarust::plugin::log;
use crate::store_state::PluginStoreState;

pub(crate) fn enabled_level() -> Option<Level> {
    LevelFilter::current().min(STATIC_MAX_LEVEL).into_level()
}

fn level_to_wit(level: Level) -> log::Level {
    match level {
        Level::ERROR => log::Level::Error,
        Level::WARN => log::Level::Warn,
        Level::INFO => log::Level::Info,
        Level::DEBUG => log::Level::Debug,
        _ => log::Level::Trace,
    }
}

impl log::Host for PluginStoreState {
    async fn max_level(&mut self) -> wasmtime::Result<Option<log::Level>> {
        Ok(enabled_level().map(level_to_wit))
    }

    async fn trace(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::trace!(plugin = %self.plugin_id(), "{message}");
        Ok(())
    }

    async fn debug(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::debug!(plugin = %self.plugin_id(), "{message}");
        Ok(())
    }

    async fn info(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::info!(plugin = %self.plugin_id(), "{message}");
        Ok(())
    }

    async fn warn(&mut self, message: String) -> wasmtime::Result<()> {
        tracing::warn!(plugin = %self.plugin_id(), "{message}");
        Ok(())
    }

    async fn error(&mut self, message: String) -> wasmtime::Result<()> {
        if is_guest_panic_line(&message) {
            tracing::debug!(plugin = %self.plugin_id(), "{message}");
            self.record_guest_panic(message);
            return Ok(());
        }
        tracing::error!(plugin = %self.plugin_id(), "{message}");
        Ok(())
    }
}
