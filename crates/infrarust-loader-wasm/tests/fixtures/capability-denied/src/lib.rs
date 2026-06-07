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

fixture_common::raw_fixture!(
    Component,
    id: "capability-denied",
    name: "Capability Denied Fixture",
    description: None,
    on_enable: {
        let _ = ban_service::is_banned(&BanTarget::Username("nobody".to_string()));
        Ok(())
    }
);

export!(Component);
