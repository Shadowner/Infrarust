//! WASM-1 `hello` fixture: loads, enables and disables cleanly. Writes a marker file into
//! its WASI-scoped data_dir so the host test can confirm `on_enable` actually ran.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use exports::infrarust::plugin::guest::{Event, EventOutcome, Guest, PluginMetadata};

struct Component;

impl Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "hello".to_string(),
            name: "Hello Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: Some("WASM-1 hello fixture".to_string()),
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        // The host preopens data_dir as the guest root; this lands in plugins/hello/.
        std::fs::write("enabled.marker", b"on_enable ran").map_err(|e| e.to_string())?;
        Ok(())
    }

    fn on_disable() -> Result<(), String> {
        let _ = std::fs::write("disabled.marker", b"on_disable ran");
        Ok(())
    }

    fn handle_event(_ev: Event) -> EventOutcome {
        EventOutcome::None
    }

    fn handle_command(_callback_id: u64, _args: Vec<String>, _player: Option<u64>) {}

    fn tab_complete(_callback_id: u64, _partial: Vec<String>) -> Vec<String> {
        vec![]
    }

    fn on_scheduled_task(_callback_id: u64) {}
}

export!(Component);
