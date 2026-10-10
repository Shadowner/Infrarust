use std::net::SocketAddr;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};
use infrarust_api::types::{Component, ProtocolVersion, ServerId};

use crate::event_bus::{CORE_OWNER, EventBusImpl};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{ConnectionIntent, HandshakeData, Refused, RoutingData};

pub const DENIED_MESSAGE: &str = "You are not allowed to connect to this server";

#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    Admitted,
    Denied(Component),
    Dropped,
}

pub fn reject(
    bus: &EventBusImpl,
    remote_addr: SocketAddr,
    virtual_host: Option<String>,
    reason: RejectReason,
) {
    if bus.has_listeners::<ConnectionRejectedEvent>() {
        bus.post(ConnectionRejectedEvent::new(
            remote_addr,
            virtual_host,
            reason,
        ));
    }
}

pub fn reject_refused(bus: &EventBusImpl, ctx: &ConnectionContext) {
    if let Some(Refused(reason)) = ctx.extensions.get::<Refused>() {
        let virtual_host = ctx
            .extensions
            .get::<HandshakeData>()
            .map(|h| h.domain.clone());
        reject(bus, ctx.client_addr(), virtual_host, reason.clone());
    }
}

pub fn handshake_event(
    bus: &EventBusImpl,
    build: impl FnOnce() -> ConnectionHandshakeEvent,
) -> Option<ConnectionHandshakeEvent> {
    bus.has_listeners::<ConnectionHandshakeEvent>().then(build)
}

pub async fn screen(
    bus: &EventBusImpl,
    build: impl FnOnce() -> ConnectionHandshakeEvent,
) -> Admission {
    let Some(event) = handshake_event(bus, build) else {
        return Admission::Admitted;
    };
    let (event, decided_by) = bus.fire_decided(event).await;
    let admission = match event.result() {
        ConnectionHandshakeResult::Deny { reason } => Admission::Denied(
            reason
                .clone()
                .unwrap_or_else(|| Component::text(DENIED_MESSAGE)),
        ),
        ConnectionHandshakeResult::DropSilently => Admission::Dropped,
        _ => return Admission::Admitted,
    };
    let plugin_id = decided_by
        .filter(|owner| &**owner != CORE_OWNER)
        .map(|owner| owner.to_string());
    reject(
        bus,
        event.remote_addr,
        event.virtual_host,
        RejectReason::Plugin { plugin_id },
    );
    admission
}

pub async fn screen_connection(bus: &EventBusImpl, ctx: &ConnectionContext) -> Admission {
    screen(bus, || {
        let handshake = ctx.extensions.get::<HandshakeData>();
        let intent = handshake.map_or(HandshakeIntent::Login, |h| match h.intent {
            ConnectionIntent::Status => HandshakeIntent::Status,
            ConnectionIntent::Login => HandshakeIntent::Login,
            ConnectionIntent::Transfer => HandshakeIntent::Transfer,
        });
        let protocol_version = handshake.map_or(0, |h| h.protocol_version.0);
        let event = ConnectionHandshakeEvent::new(
            ctx.client_addr(),
            intent,
            ProtocolVersion::new(protocol_version),
        )
        .with_server(
            ctx.extensions
                .get::<RoutingData>()
                .map(|routing| ServerId::new(routing.config_id.clone())),
        );
        match handshake {
            Some(h) => event.with_host(h.raw_host.clone(), Some(h.domain.clone()), h.port),
            None => event,
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::cell::Cell;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};

    use super::*;

    fn build(built: &Cell<usize>) -> ConnectionHandshakeEvent {
        built.set(built.get() + 1);
        ConnectionHandshakeEvent::new(
            "127.0.0.1:40000".parse().unwrap(),
            HandshakeIntent::Login,
            ProtocolVersion::MINECRAFT_1_21,
        )
    }

    #[test]
    fn the_handshake_event_is_only_built_while_someone_listens() {
        let bus = EventBusImpl::new();
        let built = Cell::new(0);

        assert!(handshake_event(&bus, || build(&built)).is_none());
        assert_eq!(built.get(), 0);

        let listeners: &dyn EventBus = &bus;
        let handle =
            listeners.subscribe::<ConnectionHandshakeEvent, _>(EventPriority::NORMAL, |_| {});
        assert!(handshake_event(&bus, || build(&built)).is_some());
        assert_eq!(built.get(), 1);

        listeners.unsubscribe(handle);
        assert!(handshake_event(&bus, || build(&built)).is_none());
        assert_eq!(built.get(), 1);
    }

    #[tokio::test]
    async fn an_unheard_handshake_admits_without_building_the_event() {
        let bus = EventBusImpl::new();
        let built = Cell::new(0);

        assert_eq!(screen(&bus, || build(&built)).await, Admission::Admitted);
        assert_eq!(built.get(), 0);
    }

    #[tokio::test]
    async fn a_deny_without_reason_gets_the_default_message() {
        let bus = EventBusImpl::new();
        let listeners: &dyn EventBus = &bus;
        listeners.subscribe::<ConnectionHandshakeEvent, _>(EventPriority::NORMAL, |event| {
            event.set_result(ConnectionHandshakeResult::Deny { reason: None });
        });
        let built = Cell::new(0);

        assert_eq!(
            screen(&bus, || build(&built)).await,
            Admission::Denied(Component::text(DENIED_MESSAGE))
        );
    }
}
