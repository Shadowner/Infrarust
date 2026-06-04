//! WASM-2 `event-subscriber` fixture: subscribes to `post-login` in `on_enable`
//! and writes the joining player's username to its data_dir on receipt, so the
//! host test can confirm the event reached the guest.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use exports::infrarust::plugin::guest::{Event, EventOutcome, Guest, PluginMetadata};

use crate::infrarust::plugin::event_bus::{self, EventKind};
use crate::infrarust::plugin::types::EventPriority;

struct Component;

impl Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "event-subscriber".to_string(),
            name: "Event Subscriber Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        event_bus::subscribe(EventKind::PostLogin, EventPriority::Normal);
        Ok(())
    }

    fn on_disable() -> Result<(), String> {
        Ok(())
    }

    fn handle_event(ev: Event) -> EventOutcome {
        if let Event::PostLogin(e) = ev {
            let _ = std::fs::write("post-login.marker", e.profile.username.as_bytes());
        }
        EventOutcome::None
    }

    fn handle_command(_callback_id: u64, _args: Vec<String>, _player: Option<u64>) {}

    fn tab_complete(_callback_id: u64, _partial: Vec<String>) -> Vec<String> {
        vec![]
    }

    fn on_scheduled_task(_callback_id: u64) {}
}

export!(Component);
