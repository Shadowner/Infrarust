wit_bindgen::generate!({
    world: "plugin",
    path: "wit",
    generate_all,
});

use crate::exports::infrarust::plugin::codec_filter::{
    self, CodecSessionInit, ConnectionState, FilterOutput, GuestFilterInstance,
};
use crate::exports::infrarust::plugin::guest::{
    self, Event, EventOutcome, HandlerResult, LimboSession, PermissionLevel, PluginMetadata,
    SessionEndReason,
};

struct Component;

impl guest::Guest for Component {
    fn metadata() -> PluginMetadata {
        PluginMetadata {
            id: "old-world".to_owned(),
            name: "Old World Fixture".to_owned(),
            version: "0.1.0".to_owned(),
            authors: Vec::new(),
            description: None,
            dependencies: Vec::new(),
        }
    }
    fn on_enable() -> Result<(), String> {
        Ok(())
    }
    fn on_disable() -> Result<(), String> {
        Ok(())
    }
    fn handle_event(_listener: u64, _ev: Event) -> EventOutcome {
        EventOutcome::None
    }
    fn handle_command(_callback_id: u64, _args: Vec<String>, _player: Option<u64>) {}
    fn tab_complete(_callback_id: u64, _partial: Vec<String>, _cursor: u32) -> Vec<String> {
        Vec::new()
    }
    fn on_scheduled_task(_callback_id: u64) {}
    fn limbo_on_player_enter(_handler: u64, _session: &LimboSession) -> HandlerResult {
        HandlerResult::Accept
    }
    fn limbo_on_command(
        _handler: u64,
        _session: &LimboSession,
        _command: String,
        _args: Vec<String>,
    ) {
    }
    fn limbo_on_chat(_handler: u64, _session: &LimboSession, _message: String) {}
    fn limbo_on_disconnect(_handler: u64, _player: u64) {}
    fn limbo_on_session_end(_handler: u64, _player: u64, _reason: SessionEndReason) {}
    fn permission_level_of(_handler: u64) -> PermissionLevel {
        PermissionLevel::Player
    }
    fn check_permission(_handler: u64, _permission: String) -> bool {
        false
    }
}

struct Passthrough;

impl codec_filter::Guest for Component {
    type FilterInstance = Passthrough;
    fn create(_factory: u64, _init: CodecSessionInit) -> codec_filter::FilterInstance {
        codec_filter::FilterInstance::new(Passthrough)
    }
}

impl GuestFilterInstance for Passthrough {
    fn filter(&self, _packet_id: i32, _data: Vec<u8>) -> FilterOutput {
        FilterOutput::Pass
    }
    fn on_state_change(&self, _new_state: ConnectionState) {}
    fn on_compression_change(&self, _threshold: i32) {}
    fn on_encryption_enabled(&self) {}
    fn on_close(&self) {}
}

export!(Component);
