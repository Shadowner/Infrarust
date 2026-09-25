use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::{EventPriority, PacketFilter};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::permissions::Capability;

use crate::actor::CallKind;
use crate::bindings::infrarust::plugin::event_bus;
use crate::bindings::infrarust::plugin::events::{EventKind, NamedEventResult};
use crate::bindings::infrarust::plugin::types::ErrorKind;
use crate::convert;
use crate::events::{self, Registration};
use crate::host_error::{HostResult, host_error, timed_out};
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

    async fn subscribe_named(
        &mut self,
        name: String,
        priority: u8,
    ) -> wasmtime::Result<HostResult<u64>> {
        Ok(self.subscribe_named_event(name, priority))
    }

    async fn fire_named(
        &mut self,
        name: String,
        content_type: String,
        payload: Vec<u8>,
    ) -> wasmtime::Result<HostResult<NamedEventResult>> {
        Ok(self.fire_named_event(name, content_type, payload).await)
    }

    async fn subscribe_packets(
        &mut self,
        filters: Vec<event_bus::PacketFilter>,
        priority: u8,
    ) -> wasmtime::Result<HostResult<u64>> {
        Ok(self.subscribe_packet_filters(&filters, priority))
    }
}

impl PluginStoreState {
    fn subscribe_event(&mut self, kind: EventKind, priority: u8) -> HostResult<u64> {
        self.check(Capability::EventBus, "event-bus.subscribe")?;
        match kind {
            EventKind::ChatMessage => self.check(
                Capability::ChatIntercept,
                "event-bus.subscribe(chat-message)",
            )?,
            EventKind::CommandExecute => self.check(
                Capability::ChatIntercept,
                "event-bus.subscribe(command-execute)",
            )?,
            EventKind::PluginMessage => self.check(
                Capability::PluginMessaging,
                "event-bus.subscribe(plugin-message)",
            )?,
            EventKind::RawPacket => {
                self.check(Capability::RawPacket, "event-bus.subscribe(raw-packet)")?
            }
            _ => {}
        }
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Event);
        let listener = self.mint_listener_id();
        match events::register(
            ctx.event_bus(),
            instance,
            kind,
            EventPriority::custom(priority),
            listener,
        ) {
            Registration::Registered(handle) => {
                self.record_listener(listener, vec![handle]);
                Ok(listener)
            }
            Registration::Refused(reason) => Err(host_error(ErrorKind::InvalidArgument, reason)),
        }
    }

    fn subscribe_named_event(&mut self, name: String, priority: u8) -> HostResult<u64> {
        self.check(Capability::EventBus, "event-bus.subscribe-named")?;
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Event);
        let listener = self.mint_listener_id();
        let handle = events::register_named(
            ctx.event_bus(),
            instance,
            name,
            EventPriority::custom(priority),
            listener,
        );
        self.record_listener(listener, vec![handle]);
        Ok(listener)
    }

    async fn fire_named_event(
        &mut self,
        name: String,
        content_type: String,
        payload: Vec<u8>,
    ) -> HostResult<NamedEventResult> {
        self.check(Capability::EventBus, "event-bus.fire-named")?;
        let ctx = self.services()?;
        let limit = self.service_call_limit();
        let event = NamedEvent::new(name, content_type, payload);
        match limit.run(ctx.event_bus().fire(event)).await {
            Ok(Ok(event)) => Ok(events::named_result(&event)),
            Ok(Err(refused)) => Err(host_error(ErrorKind::PermissionDenied, refused.to_string())),
            Err(expired) => Err(timed_out(expired)),
        }
    }

    fn subscribe_packet_filters(
        &mut self,
        filters: &[event_bus::PacketFilter],
        priority: u8,
    ) -> HostResult<u64> {
        self.check(Capability::EventBus, "event-bus.subscribe-packets")?;
        self.check(Capability::RawPacket, "event-bus.subscribe-packets")?;
        if filters.is_empty() {
            return Err(host_error(
                ErrorKind::InvalidArgument,
                "subscribe-packets needs at least one packet filter",
            ));
        }
        let filters: Vec<PacketFilter> = filters
            .iter()
            .map(|filter| PacketFilter {
                packet_id: filter.packet_id,
                state: convert::connection_state_from_wit(filter.state),
                direction: convert::packet_direction_from_wit(filter.direction),
            })
            .collect();
        let ctx = self.services()?;
        let instance = self.instance_ref(CallKind::Event);
        let listener = self.mint_listener_id();
        let handles = events::register_packets(
            ctx.event_bus(),
            &instance,
            &filters,
            EventPriority::custom(priority),
            listener,
        );
        self.record_listener(listener, handles);
        Ok(listener)
    }

    fn unsubscribe_event(&mut self, handle: u64) -> HostResult<bool> {
        self.check(Capability::EventBus, "event-bus.unsubscribe")?;
        let Some(native) = self.take_listener(handle) else {
            return Ok(false);
        };
        if let Ok(ctx) = self.services() {
            for handle in native {
                ctx.event_bus().unsubscribe(handle);
            }
        }
        Ok(true)
    }
}
