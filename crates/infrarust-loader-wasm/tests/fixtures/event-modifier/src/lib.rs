//! WASM-2 `event-modifier` fixture: subscribes to `server-pre-connect` and
//! redirects every connection to "backend-1", so the host test can confirm the
//! outcome is applied back onto the native event.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use exports::infrarust::plugin::guest::{
    Event, EventOutcome, Guest, PluginMetadata, ServerPreConnectResult,
};

use crate::infrarust::plugin::event_bus::{self, EventKind};
use crate::infrarust::plugin::types::EventPriority;

struct Component;

impl Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "event-modifier".to_string(),
            name: "Event Modifier Fixture".to_string(),
            version: "0.1.0".to_string(),
            authors: vec![],
            description: None,
            dependencies: vec![],
        }
    }

    fn on_enable() -> Result<(), String> {
        event_bus::subscribe(EventKind::ServerPreConnect, EventPriority::Normal);
        Ok(())
    }

    fn on_disable() -> Result<(), String> {
        Ok(())
    }

    fn handle_event(ev: Event) -> EventOutcome {
        match ev {
            Event::ServerPreConnect(_) => {
                EventOutcome::ServerPreConnect(ServerPreConnectResult::ConnectTo(
                    "backend-1".to_string(),
                ))
            }
            _ => EventOutcome::None,
        }
    }

    fn handle_command(_callback_id: u64, _args: Vec<String>, _player: Option<u64>) {}

    fn tab_complete(_callback_id: u64, _partial: Vec<String>) -> Vec<String> {
        vec![]
    }

    fn on_scheduled_task(_callback_id: u64) {}
}

export!(Component);
