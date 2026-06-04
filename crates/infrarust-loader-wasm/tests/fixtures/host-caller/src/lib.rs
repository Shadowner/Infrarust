//! WASM-2 `host-caller` fixture: calls read-only host services in `on_enable`
//! and records the results into its data_dir so the host test can assert them.

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
            id: "host-caller".to_string(),
            name: "Host Caller Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        let count = crate::infrarust::plugin::player_registry::online_count();
        std::fs::write("count.txt", count.to_string().as_bytes()).map_err(|e| e.to_string())?;
        if let Some(greeting) = crate::infrarust::plugin::config_service::get_value("greeting") {
            std::fs::write("greeting.txt", greeting.as_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
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
