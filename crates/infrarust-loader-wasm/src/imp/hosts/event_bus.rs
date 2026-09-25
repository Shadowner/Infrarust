use infrarust_api::event::EventPriority;
use infrarust_api::permissions::Capability;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::event_bus;
use crate::bindings::infrarust::plugin::events::EventKind;
use crate::events;
use crate::host_error::HostResult;
use crate::store_state::PluginStoreState;

impl event_bus::Host for PluginStoreState {
    async fn subscribe(
        &mut self,
        kind: EventKind,
        priority: u8,
    ) -> wasmtime::Result<HostResult<u64>> {
        Ok(self.subscribe_event(kind, priority))
    }

    async fn unsubscribe(&mut self, handle: u64) -> wasmtime::Result<HostResult<bool>> {
        Ok(self.unsubscribe_event(handle))
    }
}

impl PluginStoreState {
    fn subscribe_event(&mut self, kind: EventKind, priority: u8) -> HostResult<u64> {
        self.check(Capability::EventBus, "event-bus.subscribe")?;
        if kind == EventKind::ChatMessage {
            self.check(
                Capability::ChatIntercept,
                "event-bus.subscribe(chat-message)",
            )?;
        }
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Event);
        let listener = self.mint_listener_id();
        let handle = events::register(
            ctx.event_bus(),
            instance,
            kind,
            EventPriority::custom(priority),
            listener,
        );
        self.record_listener(listener, handle);
        Ok(listener)
    }

    fn unsubscribe_event(&mut self, handle: u64) -> HostResult<bool> {
        self.check(Capability::EventBus, "event-bus.unsubscribe")?;
        let Some(native) = self.take_listener(handle) else {
            return Ok(false);
        };
        if let Ok(ctx) = self.services() {
            ctx.event_bus().unsubscribe(native);
        }
        Ok(true)
    }
}
