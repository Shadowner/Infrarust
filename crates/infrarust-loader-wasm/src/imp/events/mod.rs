mod admin;
mod chat;
mod client;
mod connection;
mod handshake;
mod limbo;
mod login;
mod messaging;
mod packet;
mod proxy;

pub(crate) use messaging::named_result;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority, ListenerHandle, PacketFilter};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
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
use infrarust_api::events::messaging::PluginMessageEvent;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::events::resource_pack::PlayerResourcePackStatusEvent;
use infrarust_api::events::transfer::PreTransferEvent;
use infrarust_api::types::Component;
use infrarust_plugin_wit::arena::ArenaError;

use crate::actor::InstanceRef;
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::plugin::call_guest;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    Unchanged,
    Set,
    Fallback(ArenaError),
    Mismatched,
}

pub(crate) trait WasmEvent: Send + 'static {
    const KIND: EventKind;

    fn to_wit(&self) -> we::Event;

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        unmatched(&outcome)
    }
}

pub(crate) fn unmatched(outcome: &we::EventOutcome) -> Applied {
    if matches!(outcome, we::EventOutcome::Unchanged) {
        Applied::Unchanged
    } else {
        Applied::Mismatched
    }
}

#[derive(Default)]
pub(crate) struct Texts {
    failure: Option<ArenaError>,
}

impl Texts {
    pub(crate) fn convert(&mut self, text: &wt::Component) -> Component {
        component::from_wit(text).unwrap_or_else(|error| {
            self.failure.get_or_insert(error);
            component::fallback()
        })
    }

    pub(crate) fn applied(self) -> Applied {
        self.failure.map_or(Applied::Set, Applied::Fallback)
    }
}

pub(crate) enum Registration {
    Refused(&'static str),
    Registered(ListenerHandle),
}

pub(crate) fn register(
    bus: &dyn EventBus,
    instance: InstanceRef,
    kind: EventKind,
    priority: EventPriority,
    listener: u64,
) -> Registration {
    let at = priority;
    let handle = match kind {
        EventKind::PreLogin => subscribe::<PreLoginEvent>(bus, instance, at, listener),
        EventKind::PostLogin => subscribe::<PostLoginEvent>(bus, instance, at, listener),
        EventKind::Disconnect => subscribe::<DisconnectEvent>(bus, instance, at, listener),
        EventKind::OnlineAuthFailed => subscribe::<OnlineAuthFailed>(bus, instance, at, listener),
        EventKind::PermissionsSetup => {
            subscribe::<PermissionsSetupEvent>(bus, instance, at, listener)
        }
        EventKind::PlayerChooseInitialServer => {
            subscribe::<PlayerChooseInitialServerEvent>(bus, instance, at, listener)
        }
        EventKind::ServerPreConnect => {
            subscribe::<ServerPreConnectEvent>(bus, instance, at, listener)
        }
        EventKind::ServerConnected => {
            subscribe::<ServerConnectedEvent>(bus, instance, at, listener)
        }
        EventKind::ServerPostConnect => {
            subscribe::<ServerPostConnectEvent>(bus, instance, at, listener)
        }
        EventKind::KickedFromServer => {
            subscribe::<KickedFromServerEvent>(bus, instance, at, listener)
        }
        EventKind::ChatMessage => subscribe::<ChatMessageEvent>(bus, instance, at, listener),
        EventKind::ProxyPing => subscribe::<ProxyPingEvent>(bus, instance, at, listener),
        EventKind::ProxyInitialize => {
            subscribe::<ProxyInitializeEvent>(bus, instance, at, listener)
        }
        EventKind::ProxyShutdown => subscribe::<ProxyShutdownEvent>(bus, instance, at, listener),
        EventKind::ConfigReload => subscribe::<ConfigReloadEvent>(bus, instance, at, listener),
        EventKind::ServerStateChange => {
            subscribe::<ServerStateChangeEvent>(bus, instance, at, listener)
        }
        EventKind::BackendHealth => subscribe::<BackendHealthEvent>(bus, instance, at, listener),
        EventKind::Login => subscribe::<LoginEvent>(bus, instance, at, listener),
        EventKind::GameProfileRequest => {
            subscribe::<GameProfileRequestEvent>(bus, instance, at, listener)
        }
        EventKind::CommandExecute => subscribe::<CommandExecuteEvent>(bus, instance, at, listener),
        EventKind::ConnectionHandshake => {
            subscribe::<ConnectionHandshakeEvent>(bus, instance, at, listener)
        }
        EventKind::ConnectionRejected => {
            subscribe::<ConnectionRejectedEvent>(bus, instance, at, listener)
        }
        EventKind::LimboEnter => subscribe::<LimboEnterEvent>(bus, instance, at, listener),
        EventKind::LimboExit => subscribe::<LimboExitEvent>(bus, instance, at, listener),
        EventKind::PlayerClientBrand => {
            subscribe::<PlayerClientBrandEvent>(bus, instance, at, listener)
        }
        EventKind::PlayerSettingsChanged => {
            subscribe::<PlayerSettingsChangedEvent>(bus, instance, at, listener)
        }
        EventKind::PlayerChannelRegister => {
            subscribe::<PlayerChannelRegisterEvent>(bus, instance, at, listener)
        }
        EventKind::PluginMessage => subscribe::<PluginMessageEvent>(bus, instance, at, listener),
        EventKind::BanIssued => subscribe::<BanIssuedEvent>(bus, instance, at, listener),
        EventKind::BanRevoked => subscribe::<BanRevokedEvent>(bus, instance, at, listener),
        EventKind::PluginEnabled => subscribe::<PluginEnabledEvent>(bus, instance, at, listener),
        EventKind::PluginDisabled => subscribe::<PluginDisabledEvent>(bus, instance, at, listener),
        EventKind::PreTransfer => subscribe::<PreTransferEvent>(bus, instance, at, listener),
        EventKind::PlayerResourcePackStatus => {
            subscribe::<PlayerResourcePackStatusEvent>(bus, instance, at, listener)
        }
        EventKind::NamedEvent => subscribe::<NamedEvent>(bus, instance, at, listener),
        EventKind::RawPacket => {
            return Registration::Refused(
                "raw packets are subscribed with event-bus.subscribe-packets and a packet filter",
            );
        }
    };
    Registration::Registered(handle)
}

pub(crate) fn register_named(
    bus: &dyn EventBus,
    instance: InstanceRef,
    name: String,
    priority: EventPriority,
    listener: u64,
) -> ListenerHandle {
    bus.subscribe_async::<NamedEvent, _>(priority, move |event: &mut NamedEvent| {
        if event.name == name {
            deliver(event, instance.clone(), listener)
        } else {
            Box::pin(async {})
        }
    })
}

pub(crate) fn register_packets(
    bus: &dyn EventBus,
    instance: &InstanceRef,
    filters: &[PacketFilter],
    priority: EventPriority,
    listener: u64,
) -> Vec<ListenerHandle> {
    filters
        .iter()
        .map(|filter| {
            let instance = instance.clone();
            bus.subscribe_packet_async(
                *filter,
                priority,
                Box::new(move |any| match any.downcast_mut::<RawPacketEvent>() {
                    Some(event) => deliver(event, instance.clone(), listener),
                    None => Box::pin(async {}),
                }),
            )
        })
        .collect()
}

fn subscribe<E: WasmEvent + Event>(
    bus: &dyn EventBus,
    instance: InstanceRef,
    priority: EventPriority,
    listener: u64,
) -> ListenerHandle {
    bus.subscribe_async::<E, _>(priority, move |event: &mut E| {
        deliver(event, instance.clone(), listener)
    })
}

fn deliver<E: WasmEvent>(event: &mut E, instance: InstanceRef, listener: u64) -> BoxFuture<'_, ()> {
    let wit = event.to_wit();
    if instance.is_upstream() {
        post(&instance, E::KIND, listener, wit);
        return Box::pin(async {});
    }
    Box::pin(async move {
        let outcome = call_guest(instance.clone(), "handle-event", move |store, bindings| {
            Box::pin(async move {
                bindings
                    .infrarust_plugin_guest()
                    .call_handle_event(&mut *store, listener, &wit)
                    .await
            })
        })
        .await;
        if let Some(outcome) = outcome {
            settle(event, outcome, &instance);
        }
    })
}

fn post(instance: &InstanceRef, kind: EventKind, listener: u64, wit: we::Event) {
    if let Some(suppressed) = instance.admit_warning() {
        tracing::warn!(
            plugin = instance.plugin_id(),
            event = kind_name(kind),
            suppressed,
            "wasm plugin is still running the call that led to this event; it receives the \
             event without the proxy waiting for it, and its answer is ignored"
        );
    }
    let _ = instance.post("handle-event", move |store, bindings| {
        Box::pin(async move {
            bindings
                .infrarust_plugin_guest()
                .call_handle_event(&mut *store, listener, &wit)
                .await
        })
    });
}

fn settle<E: WasmEvent>(event: &mut E, outcome: we::EventOutcome, instance: &InstanceRef) {
    let answered = outcome_name(&outcome);
    let plugin = instance.plugin_id();
    let event_name = kind_name(E::KIND);
    match event.apply(outcome) {
        Applied::Unchanged | Applied::Set => {}
        Applied::Fallback(error) => {
            if let Some(suppressed) = instance.admit_warning() {
                tracing::warn!(plugin, event = event_name, %error, suppressed,
                    "wasm plugin set a result with an invalid text component; the result applies with a fallback text");
            }
        }
        Applied::Mismatched => {
            if let Some(suppressed) = instance.admit_warning() {
                tracing::warn!(
                    plugin,
                    event = event_name,
                    answered,
                    suppressed,
                    "wasm plugin answered an event with the outcome of another event; ignoring it"
                );
            }
        }
    }
}

pub(crate) const fn kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::PreLogin => "pre-login",
        EventKind::PostLogin => "post-login",
        EventKind::Disconnect => "disconnect",
        EventKind::OnlineAuthFailed => "online-auth-failed",
        EventKind::PermissionsSetup => "permissions-setup",
        EventKind::PlayerChooseInitialServer => "player-choose-initial-server",
        EventKind::ServerPreConnect => "server-pre-connect",
        EventKind::ServerConnected => "server-connected",
        EventKind::ServerPostConnect => "server-post-connect",
        EventKind::KickedFromServer => "kicked-from-server",
        EventKind::ChatMessage => "chat-message",
        EventKind::ProxyPing => "proxy-ping",
        EventKind::ProxyInitialize => "proxy-initialize",
        EventKind::ProxyShutdown => "proxy-shutdown",
        EventKind::ConfigReload => "config-reload",
        EventKind::ServerStateChange => "server-state-change",
        EventKind::BackendHealth => "backend-health",
        EventKind::Login => "login",
        EventKind::GameProfileRequest => "game-profile-request",
        EventKind::CommandExecute => "command-execute",
        EventKind::ConnectionHandshake => "connection-handshake",
        EventKind::ConnectionRejected => "connection-rejected",
        EventKind::LimboEnter => "limbo-enter",
        EventKind::LimboExit => "limbo-exit",
        EventKind::PlayerClientBrand => "player-client-brand",
        EventKind::PlayerSettingsChanged => "player-settings-changed",
        EventKind::PlayerChannelRegister => "player-channel-register",
        EventKind::PluginMessage => "plugin-message",
        EventKind::BanIssued => "ban-issued",
        EventKind::BanRevoked => "ban-revoked",
        EventKind::PluginEnabled => "plugin-enabled",
        EventKind::PluginDisabled => "plugin-disabled",
        EventKind::PreTransfer => "pre-transfer",
        EventKind::PlayerResourcePackStatus => "player-resource-pack-status",
        EventKind::NamedEvent => "named-event",
        EventKind::RawPacket => "raw-packet",
    }
}

const fn outcome_name(outcome: &we::EventOutcome) -> &'static str {
    match outcome {
        we::EventOutcome::Unchanged => "unchanged",
        we::EventOutcome::PreLogin(_) => "pre-login",
        we::EventOutcome::PermissionsSetup(_) => "permissions-setup",
        we::EventOutcome::PlayerChooseInitialServer(_) => "player-choose-initial-server",
        we::EventOutcome::ServerPreConnect(_) => "server-pre-connect",
        we::EventOutcome::KickedFromServer(_) => "kicked-from-server",
        we::EventOutcome::ChatMessage(_) => "chat-message",
        we::EventOutcome::ProxyPing(_) => "proxy-ping",
        we::EventOutcome::Login(_) => "login",
        we::EventOutcome::GameProfileRequest(_) => "game-profile-request",
        we::EventOutcome::CommandExecute(_) => "command-execute",
        we::EventOutcome::ConnectionHandshake(_) => "connection-handshake",
        we::EventOutcome::PluginMessage(_) => "plugin-message",
        we::EventOutcome::PreTransfer(_) => "pre-transfer",
        we::EventOutcome::NamedEvent(_) => "named-event",
        we::EventOutcome::RawPacket(_) => "raw-packet",
    }
}

#[cfg(test)]
pub(crate) fn steve() -> std::sync::Arc<dyn infrarust_api::player::Player> {
    let (player, _commands) = infrarust_core::player::PlayerSession::new_test(true);
    std::sync::Arc::new(player)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use infrarust_core::event_bus::EventBusImpl;
    use infrarust_core::plugin::tracking::TrackingEventBus;

    use super::*;

    const ALL_KINDS: [EventKind; 35] = [
        EventKind::PreLogin,
        EventKind::PostLogin,
        EventKind::Disconnect,
        EventKind::OnlineAuthFailed,
        EventKind::PermissionsSetup,
        EventKind::PlayerChooseInitialServer,
        EventKind::ServerPreConnect,
        EventKind::ServerConnected,
        EventKind::ServerPostConnect,
        EventKind::KickedFromServer,
        EventKind::ChatMessage,
        EventKind::ProxyPing,
        EventKind::ProxyInitialize,
        EventKind::ProxyShutdown,
        EventKind::ConfigReload,
        EventKind::ServerStateChange,
        EventKind::BackendHealth,
        EventKind::Login,
        EventKind::GameProfileRequest,
        EventKind::CommandExecute,
        EventKind::ConnectionHandshake,
        EventKind::ConnectionRejected,
        EventKind::LimboEnter,
        EventKind::LimboExit,
        EventKind::PlayerClientBrand,
        EventKind::PlayerSettingsChanged,
        EventKind::PlayerChannelRegister,
        EventKind::PluginMessage,
        EventKind::BanIssued,
        EventKind::BanRevoked,
        EventKind::PluginEnabled,
        EventKind::PluginDisabled,
        EventKind::PreTransfer,
        EventKind::PlayerResourcePackStatus,
        EventKind::NamedEvent,
    ];

    #[test]
    fn every_event_kind_registers_one_listener() {
        let bus = TrackingEventBus::new(Arc::new(EventBusImpl::new()), "guest");
        for (at, kind) in ALL_KINDS.into_iter().enumerate() {
            let registration = register(
                &bus,
                InstanceRef::detached(),
                kind,
                EventPriority::NORMAL,
                1,
            );
            assert!(
                matches!(registration, Registration::Registered(_)),
                "{}",
                kind_name(kind)
            );
            assert_eq!(bus.tracked_count(), at + 1, "{}", kind_name(kind));
        }
    }

    #[test]
    fn raw_packets_need_a_packet_filter() {
        let bus = TrackingEventBus::new(Arc::new(EventBusImpl::new()), "guest");
        let registration = register(
            &bus,
            InstanceRef::detached(),
            EventKind::RawPacket,
            EventPriority::NORMAL,
            1,
        );
        assert!(matches!(registration, Registration::Refused(_)));
        assert_eq!(bus.tracked_count(), 0);

        let filters = [
            PacketFilter {
                packet_id: 3,
                state: infrarust_api::event::ConnectionState::Play,
                direction: infrarust_api::events::packet::PacketDirection::Serverbound,
            },
            PacketFilter {
                packet_id: 4,
                state: infrarust_api::event::ConnectionState::Play,
                direction: infrarust_api::events::packet::PacketDirection::Clientbound,
            },
        ];
        let handles = register_packets(
            &bus,
            &InstanceRef::detached(),
            &filters,
            EventPriority::NORMAL,
            1,
        );
        assert_eq!(handles.len(), 2);
        assert!(bus.has_packet_listeners(
            4,
            infrarust_api::event::ConnectionState::Play,
            infrarust_api::events::packet::PacketDirection::Clientbound
        ));
    }

    #[test]
    fn an_unchanged_outcome_is_not_a_mismatch_for_any_event() {
        assert_eq!(unmatched(&we::EventOutcome::Unchanged), Applied::Unchanged);
        assert_eq!(
            unmatched(&we::EventOutcome::ChatMessage(we::ChatMessageResult::Allow)),
            Applied::Mismatched
        );
    }
}
