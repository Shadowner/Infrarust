//! WASM-1 `hello` fixture, migrated to the SDK. Writes a marker into its
//! WASI-scoped data_dir so the host test can confirm `on_enable` ran.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Hello;

#[plugin(id = "hello", name = "Hello Fixture", description = "WASM-1 hello fixture")]
impl Plugin for Hello {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        // The host preopens data_dir as the guest root; this lands in plugins/hello/.
        std::fs::write("enabled.marker", b"on_enable ran").map_err(|e| e.to_string())
    }
}
