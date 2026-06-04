//! WASM-2 `capability-denied` fixture: imports AND calls `ban-service` (so the
//! import is not dead-code-eliminated) without being granted the `ban`
//! capability. The host omits the interface from the linker, so instantiation
//! must fail — the host test asserts `load()` returns `Err`.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use exports::infrarust::plugin::guest::{Event, EventOutcome, Guest, PluginMetadata};

use crate::infrarust::plugin::ban_service;
use crate::infrarust::plugin::types::BanTarget;

struct Component;

impl Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "capability-denied".to_string(),
            name: "Capability Denied Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        let _ = ban_service::is_banned(&BanTarget::Username("nobody".to_string()));
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
