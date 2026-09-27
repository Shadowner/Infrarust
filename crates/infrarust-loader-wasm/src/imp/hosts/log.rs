use infrarust_plugin_common::guest_panic::is_guest_panic_line;

use crate::bindings::infrarust::plugin::log;
use crate::store_state::PluginStoreState;

impl log::Host for PluginStoreState {
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
