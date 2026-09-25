wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../infrarust-plugin-wit/wit",
    generate_all,
});

use crate::infrarust::plugin::ban_service::{self, BanTarget};
use crate::infrarust::plugin::types::ErrorKind;

struct Component;

fn kind(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::InvalidArgument => "invalid-argument",
        ErrorKind::NotFound => "not-found",
        ErrorKind::PermissionDenied => "permission-denied",
        ErrorKind::Unavailable => "unavailable",
        ErrorKind::Timeout => "timeout",
        ErrorKind::PlayerGone => "player-gone",
        ErrorKind::Conflict => "conflict",
        ErrorKind::InvalidState => "invalid-state",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Internal => "internal",
    }
}

fn outcome() -> String {
    match ban_service::get(&BanTarget::Username("nobody".to_string())) {
        Ok(entry) => format!("ok: {}", entry.is_some()),
        Err(error) => format!("{}: {}", kind(error.kind), error.message),
    }
}

fixture_common::raw_fixture!(
    Component,
    id: "capability-denied",
    name: "Capability Denied Fixture",
    description: None,
    on_enable: {
        std::fs::write("ban.txt", outcome()).map_err(|e| e.to_string())
    }
);

export!(Component);
