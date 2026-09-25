use std::any::TypeId;

use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::connection::{
    KickedFromServerEvent, PlayerChooseInitialServerEvent, ServerConnectedEvent,
    ServerPreConnectEvent, ServerSwitchEvent,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent, PostLoginEvent, PreLoginEvent,
};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};

pub static BUILTIN_EVENTS: [TypeId; 18] = [
    TypeId::of::<PreLoginEvent>(),
    TypeId::of::<PostLoginEvent>(),
    TypeId::of::<PermissionsSetupEvent>(),
    TypeId::of::<OnlineAuthFailed>(),
    TypeId::of::<DisconnectEvent>(),
    TypeId::of::<PlayerChooseInitialServerEvent>(),
    TypeId::of::<ServerPreConnectEvent>(),
    TypeId::of::<ServerConnectedEvent>(),
    TypeId::of::<ServerSwitchEvent>(),
    TypeId::of::<KickedFromServerEvent>(),
    TypeId::of::<ChatMessageEvent>(),
    TypeId::of::<ProxyPingEvent>(),
    TypeId::of::<ProxyInitializeEvent>(),
    TypeId::of::<ProxyShutdownEvent>(),
    TypeId::of::<ConfigReloadEvent>(),
    TypeId::of::<BackendHealthEvent>(),
    TypeId::of::<ServerStateChangeEvent>(),
    TypeId::of::<RawPacketEvent>(),
];

pub fn is_builtin_event(type_id: TypeId) -> bool {
    BUILTIN_EVENTS.contains(&type_id)
}
