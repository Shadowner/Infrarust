//! WASM-2 `event-modifier` fixture, migrated to the SDK. Redirects every
//! connection to "backend-1" so the host test can confirm the outcome applies.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct EventModifier;

#[plugin(id = "event-modifier", name = "Event Modifier Fixture")]
impl Plugin for EventModifier {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            event.redirect_to("backend-1");
        });
        Ok(())
    }
}
