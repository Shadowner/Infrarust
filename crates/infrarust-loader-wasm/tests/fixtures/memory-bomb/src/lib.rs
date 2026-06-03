//! WASM-1 `memory-bomb` fixture: allocates far past the 64 MiB store limit in `on_enable`.
//! The `ResourceLimiter` refuses `memory.grow`; with `trap_on_grow_failure` that is a trap
//! the host observes as an error.

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
            id: "memory-bomb".to_string(),
            name: "Memory Bomb Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: Some("WASM-1 memory-bomb fixture".to_string()),
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        // 8 MiB chunks; after ~8 we cross the 64 MiB cap and growth is refused (a trap).
        let mut sink: Vec<Vec<u8>> = Vec::new();
        loop {
            sink.push(std::hint::black_box(vec![0u8; 8 * 1024 * 1024]));
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
