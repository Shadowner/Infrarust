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
            id: "trap-on-purpose".to_string(),
            name: "Trap On Purpose Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: Some("WASM-1 trap fixture".to_string()),
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        panic!("trap-on-purpose: intentional panic in on_enable");
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
