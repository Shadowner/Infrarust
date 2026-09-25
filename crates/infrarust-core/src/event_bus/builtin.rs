use std::any::TypeId;

use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::command::CommandExecuteEvent;
use infrarust_api::events::connection::{
    KickedFromServerEvent, PlayerChooseInitialServerEvent, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent,
};
use infrarust_api::events::handshake::{ConnectionHandshakeEvent, ConnectionRejectedEvent};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, GameProfileRequestEvent, LoginEvent, OnlineAuthFailed, PermissionsSetupEvent,
    PostLoginEvent, PreLoginEvent,
};
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};

pub static BUILTIN_EVENTS: &[TypeId] = &[
    TypeId::of::<PreLoginEvent>(),
    TypeId::of::<GameProfileRequestEvent>(),
    TypeId::of::<LoginEvent>(),
    TypeId::of::<PostLoginEvent>(),
    TypeId::of::<PermissionsSetupEvent>(),
    TypeId::of::<OnlineAuthFailed>(),
    TypeId::of::<DisconnectEvent>(),
    TypeId::of::<PlayerChooseInitialServerEvent>(),
    TypeId::of::<ServerPreConnectEvent>(),
    TypeId::of::<ServerConnectedEvent>(),
    TypeId::of::<ServerPostConnectEvent>(),
    TypeId::of::<KickedFromServerEvent>(),
    TypeId::of::<ChatMessageEvent>(),
    TypeId::of::<CommandExecuteEvent>(),
    TypeId::of::<ProxyPingEvent>(),
    TypeId::of::<ProxyInitializeEvent>(),
    TypeId::of::<ProxyShutdownEvent>(),
    TypeId::of::<ConfigReloadEvent>(),
    TypeId::of::<BackendHealthEvent>(),
    TypeId::of::<ServerStateChangeEvent>(),
    TypeId::of::<BanIssuedEvent>(),
    TypeId::of::<BanRevokedEvent>(),
    TypeId::of::<ConnectionHandshakeEvent>(),
    TypeId::of::<ConnectionRejectedEvent>(),
    TypeId::of::<LimboEnterEvent>(),
    TypeId::of::<LimboExitEvent>(),
    TypeId::of::<RawPacketEvent>(),
];

pub fn is_builtin_event(type_id: TypeId) -> bool {
    BUILTIN_EVENTS.contains(&type_id)
}
