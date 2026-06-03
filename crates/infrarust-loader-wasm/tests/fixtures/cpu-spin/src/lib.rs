//! WASM-1 `cpu-spin` fixture: spins forever in `on_enable` with no host calls, so only
//! epoch interruption can stop it. The host's escalating epoch deadline must trap it.

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
            id: "cpu-spin".to_string(),
            name: "CPU Spin Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: Some("WASM-1 cpu-spin fixture".to_string()),
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        // `black_box` keeps the loop body from being optimised out; Rust preserves
        // infinite loops regardless. Only the epoch interrupt breaks out (as a trap).
        loop {
            std::hint::black_box(0u64);
        }
    }

    fn on_disable() -> Result<(), String> {
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
