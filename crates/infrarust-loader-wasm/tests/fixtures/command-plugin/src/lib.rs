//! WASM-2 `command-plugin` fixture: registers a `greet` command in `on_enable`
//! and writes its arguments to the data_dir when invoked, so the host test can
//! confirm the command dispatch reaches the guest.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use exports::infrarust::plugin::guest::{Event, EventOutcome, Guest, PluginMetadata};

use crate::infrarust::plugin::command_manager;

const GREET_CALLBACK: u64 = 1;

struct Component;

impl Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "command-plugin".to_string(),
            name: "Command Plugin Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        command_manager::register("greet", &[], "Greets the caller", GREET_CALLBACK);
        Ok(())
    }

    fn on_disable() -> Result<(), String> {
        Ok(())
    }

    fn handle_event(_ev: Event) -> EventOutcome {
        EventOutcome::None
    }

    fn handle_command(callback_id: u64, args: Vec<String>, _player: Option<u64>) {
        if callback_id == GREET_CALLBACK {
            let _ = std::fs::write("command.marker", args.join(",").as_bytes());
        }
    }

    fn tab_complete(_callback_id: u64, _partial: Vec<String>) -> Vec<String> {
        vec![]
    }

    fn on_scheduled_task(_callback_id: u64) {}
}

export!(Component);
