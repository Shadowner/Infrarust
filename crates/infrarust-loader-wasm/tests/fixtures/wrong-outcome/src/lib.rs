wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use crate::exports::infrarust::plugin::guest::{Event, EventOutcome};
use crate::infrarust::plugin::event_bus;
use crate::infrarust::plugin::events::{ChatMessageResult, EventKind};

struct Component;

fn subscribe_all() -> Result<(), String> {
    for kind in [EventKind::PreLogin, EventKind::ChatMessage] {
        event_bus::subscribe(kind, 128).map_err(|error| error.message)?;
    }
    Ok(())
}

fn always_chat_deny(_listener: u64, _ev: Event) -> EventOutcome {
    EventOutcome::ChatMessage(ChatMessageResult::Deny(None))
}

fixture_common::raw_fixture!(
    Component,
    id: "wrong-outcome",
    name: "Wrong Outcome Fixture",
    description: None,
    on_enable: { subscribe_all() },
    handle_event: always_chat_deny,
);

export!(Component);
