//! WASM-2 `host-caller` fixture, migrated to the SDK. Calls read-only host
//! services in `on_enable` and records the results so the host test can assert.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct HostCaller;

#[plugin(id = "host-caller", name = "Host Caller Fixture")]
impl Plugin for HostCaller {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        std::fs::write("count.txt", Players.online_count().to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        if let Some(greeting) = Config.get("greeting") {
            std::fs::write("greeting.txt", greeting.as_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
