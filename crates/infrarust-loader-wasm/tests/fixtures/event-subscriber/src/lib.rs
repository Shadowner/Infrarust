//! WASM-2 `event-subscriber` fixture, migrated to the SDK. Records the joining
//! player's username on `PostLogin` so the host test can confirm delivery.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventSubscriber;

#[plugin(id = "event-subscriber", name = "Event Subscriber Fixture")]
impl Plugin for EventSubscriber {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            let _ = std::fs::write("post-login.marker", event.profile.username.as_bytes());
        });
        Ok(())
    }
}
