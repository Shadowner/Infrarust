mod admin;
mod chat;
mod client;
mod connection;
mod guard;
mod handshake;
mod limbo;
mod login;
mod messaging;
mod packet;
mod proxy;

pub(crate) use guard::{AccessListeners, guard};
pub(crate) use messaging::named_result;
pub(crate) use proxy::PingDetails;

use std::sync::Arc;

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
    DisconnectEvent, GameProfileRequestEvent, LoginEvent, OnlineAuthFailedEvent,
    PermissionsSetupEvent, PostLoginEvent, PreLoginEvent,
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
use wasmtime::Store;

use crate::actor::{CallFailure, InstanceRef};
use crate::bindings::Plugin as PluginBindings;
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::bindings::infrarust::plugin::types as wt;
use crate::chain::{CallChain, MAX_ENTRIES};
use crate::component;
use crate::snapshots::SnapshotError;
use crate::store_state::PluginStoreState;

pub(crate) const PLUGIN_UNAVAILABLE: &str =
    "A proxy plugin is unavailable. Please try again later.";

pub(crate) fn unavailable() -> Component {
    Component::text(PLUGIN_UNAVAILABLE)
}

pub(crate) struct Restore<E>(Box<dyn FnOnce(&mut E) + Send>);

impl<E> Restore<E> {
    pub(crate) fn new(restore: impl FnOnce(&mut E) + Send + 'static) -> Self {
        Self(Box::new(restore))
    }

    fn undo(self, event: &mut E) {
        (self.0)(event);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum EventDetails {
    Ping(PingDetails),
}

struct Lent<'e, E: WasmEvent> {
    event: &'e mut E,
    details: Option<Arc<EventDetails>>,
}

impl<'e, E: WasmEvent> Lent<'e, E> {
    fn new(event: &'e mut E) -> Self {
        let details = event.lend().map(Arc::new);
        Self { event, details }
    }

    fn shared(&self) -> Option<Arc<EventDetails>> {
        self.details.clone()
    }

    fn give_back(&mut self) -> &mut E {
        if let Some(details) = self.details.take() {
            let details = Arc::try_unwrap(details).unwrap_or_else(|shared| (*shared).clone());
            self.event.give_back(details);
        }
        self.event
    }
}

impl<E: WasmEvent> Drop for Lent<'_, E> {
    fn drop(&mut self) {
        self.give_back();
    }
}

fn copied<E: WasmEvent>(event: &mut E) -> Option<Arc<EventDetails>> {
    let details = event.lend()?;
    let copy = details.clone();
    event.give_back(details);
    Some(Arc::new(copy))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Applied {
    Unchanged,
    Set,
    Fallback(ArenaError),
    Degraded(SnapshotError),
    Mismatched,
}

pub(crate) trait WasmEvent: Sized + Send + 'static {
    const KIND: EventKind;

    const DENY_UNANSWERED: Option<fn(&mut Self) -> Restore<Self>> = None;

    fn to_wit(&self) -> we::Event;

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        unmatched(&outcome)
    }

    fn apply_for(&mut self, outcome: we::EventOutcome, _instance: &InstanceRef) -> Applied {
        self.apply(outcome)
    }

    fn lend(&mut self) -> Option<EventDetails> {
        None
    }

    fn give_back(&mut self, _details: EventDetails) {}
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

pub(crate) trait EventVisitor {
    type Output;

    fn visit<E: WasmEvent + Event>(self) -> Self::Output;
}

pub(crate) fn visit_kind<V: EventVisitor>(kind: EventKind, visitor: V) -> Option<V::Output> {
    Some(match kind {
        EventKind::PreLogin => visitor.visit::<PreLoginEvent>(),
        EventKind::PostLogin => visitor.visit::<PostLoginEvent>(),
        EventKind::Disconnect => visitor.visit::<DisconnectEvent>(),
        EventKind::OnlineAuthFailed => visitor.visit::<OnlineAuthFailedEvent>(),
        EventKind::PermissionsSetup => visitor.visit::<PermissionsSetupEvent>(),
        EventKind::PlayerChooseInitialServer => visitor.visit::<PlayerChooseInitialServerEvent>(),
        EventKind::ServerPreConnect => visitor.visit::<ServerPreConnectEvent>(),
        EventKind::ServerConnected => visitor.visit::<ServerConnectedEvent>(),
        EventKind::ServerPostConnect => visitor.visit::<ServerPostConnectEvent>(),
        EventKind::KickedFromServer => visitor.visit::<KickedFromServerEvent>(),
        EventKind::ChatMessage => visitor.visit::<ChatMessageEvent>(),
        EventKind::ProxyPing => visitor.visit::<ProxyPingEvent>(),
        EventKind::ProxyInitialize => visitor.visit::<ProxyInitializeEvent>(),
        EventKind::ProxyShutdown => visitor.visit::<ProxyShutdownEvent>(),
        EventKind::ConfigReload => visitor.visit::<ConfigReloadEvent>(),
        EventKind::ServerStateChange => visitor.visit::<ServerStateChangeEvent>(),
        EventKind::BackendHealth => visitor.visit::<BackendHealthEvent>(),
        EventKind::Login => visitor.visit::<LoginEvent>(),
        EventKind::GameProfileRequest => visitor.visit::<GameProfileRequestEvent>(),
        EventKind::CommandExecute => visitor.visit::<CommandExecuteEvent>(),
        EventKind::ConnectionHandshake => visitor.visit::<ConnectionHandshakeEvent>(),
        EventKind::ConnectionRejected => visitor.visit::<ConnectionRejectedEvent>(),
        EventKind::LimboEnter => visitor.visit::<LimboEnterEvent>(),
        EventKind::LimboExit => visitor.visit::<LimboExitEvent>(),
        EventKind::PlayerClientBrand => visitor.visit::<PlayerClientBrandEvent>(),
        EventKind::PlayerSettingsChanged => visitor.visit::<PlayerSettingsChangedEvent>(),
        EventKind::PlayerChannelRegister => visitor.visit::<PlayerChannelRegisterEvent>(),
        EventKind::PluginMessage => visitor.visit::<PluginMessageEvent>(),
        EventKind::BanIssued => visitor.visit::<BanIssuedEvent>(),
        EventKind::BanRevoked => visitor.visit::<BanRevokedEvent>(),
        EventKind::PluginEnabled => visitor.visit::<PluginEnabledEvent>(),
        EventKind::PluginDisabled => visitor.visit::<PluginDisabledEvent>(),
        EventKind::PreTransfer => visitor.visit::<PreTransferEvent>(),
        EventKind::PlayerResourcePackStatus => visitor.visit::<PlayerResourcePackStatusEvent>(),
        EventKind::NamedEvent => visitor.visit::<NamedEvent>(),
        EventKind::RawPacket => return None,
    })
}

struct Subscribe<'b> {
    bus: &'b dyn EventBus,
    instance: InstanceRef,
    priority: EventPriority,
    listener: u64,
}

impl EventVisitor for Subscribe<'_> {
    type Output = ListenerHandle;

    fn visit<E: WasmEvent + Event>(self) -> ListenerHandle {
        subscribe::<E>(self.bus, self.instance, self.priority, self.listener)
    }
}

pub(crate) fn register(
    bus: &dyn EventBus,
    instance: InstanceRef,
    kind: EventKind,
    priority: EventPriority,
    listener: u64,
) -> Registration {
    let subscribe = Subscribe {
        bus,
        instance,
        priority,
        listener,
    };
    match visit_kind(kind, subscribe) {
        Some(handle) => Registration::Registered(handle),
        None => Registration::Refused(
            "raw packets are subscribed with event-bus.subscribe-packets and a packet filter",
        ),
    }
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
    let tracked = E::DENY_UNANSWERED
        .is_some()
        .then(|| instance.access().track(E::KIND, priority));
    bus.subscribe_async::<E, _>(priority, move |event: &mut E| {
        let _ = &tracked;
        deliver(event, instance.clone(), listener)
    })
}

fn deliver<E: WasmEvent>(event: &mut E, instance: InstanceRef, listener: u64) -> BoxFuture<'_, ()> {
    let wit = event.to_wit();
    if instance.is_upstream() {
        let details = copied(event);
        if let Some(deny) = E::DENY_UNANSWERED
            && instance.is_reentered_by_another()
        {
            deny(event);
            report_denied_upstream(&instance, E::KIND);
        }
        post(&instance, E::KIND, listener, wit, details);
        return Box::pin(async {});
    }
    let denied = E::DENY_UNANSWERED.map(|deny| deny(event));
    Box::pin(async move {
        let mut lent = Lent::new(event);
        let details = lent.shared();
        let answer = instance
            .call("handle-event", move |store, bindings| {
                handle_event(store, bindings, listener, wit, details)
            })
            .await;
        let event = lent.give_back();
        match answer {
            Ok(outcome) => {
                if let Some(denied) = denied {
                    denied.undo(event);
                }
                settle(event, outcome, &instance);
            }
            Err(failure) if denied.is_some() => report_denied(&instance, E::KIND, &failure),
            Err(_) => {}
        }
    })
}

fn report_denied(instance: &InstanceRef, kind: EventKind, failure: &CallFailure) {
    if let Some(suppressed) = instance.admit_warning() {
        tracing::warn!(
            plugin = instance.plugin_id(),
            event = kind_name(kind),
            cause = %failure,
            suppressed,
            "access event denied: the wasm plugin listening to it did not answer"
        );
    }
}

fn report_denied_upstream(instance: &InstanceRef, kind: EventKind) {
    if let Some(suppressed) = instance.admit_warning() {
        tracing::warn!(
            plugin = instance.plugin_id(),
            event = kind_name(kind),
            suppressed,
            "access event denied: another plugin caused it inside a call that is waiting on \
             the wasm plugin listening to it, so that plugin cannot answer"
        );
    }
}

fn post(
    instance: &InstanceRef,
    kind: EventKind,
    listener: u64,
    wit: we::Event,
    details: Option<Arc<EventDetails>>,
) {
    let entries = CallChain::current().entries(instance.plugin_id());
    if entries >= MAX_ENTRIES {
        if let Some(suppressed) = instance.admit_loop_warning() {
            tracing::warn!(
                plugin = instance.plugin_id(),
                event = kind_name(kind),
                entries,
                suppressed,
                "wasm plugin event dropped: the call that led to it already entered this plugin \
                 {entries} times; plugins may be passing events back and forth"
            );
        }
        return;
    }
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
        handle_event(store, bindings, listener, wit, details)
    });
}

fn handle_event<'a>(
    store: &'a mut Store<PluginStoreState>,
    bindings: &'a PluginBindings,
    listener: u64,
    wit: we::Event,
    details: Option<Arc<EventDetails>>,
) -> BoxFuture<'a, wasmtime::Result<we::EventOutcome>> {
    Box::pin(async move {
        store.data_mut().set_event_details(details);
        bindings
            .infrarust_plugin_guest()
            .call_handle_event(&mut *store, listener, &wit)
            .await
    })
}

fn settle<E: WasmEvent>(event: &mut E, outcome: we::EventOutcome, instance: &InstanceRef) {
    let answered = outcome_name(&outcome);
    let plugin = instance.plugin_id();
    let event_name = kind_name(E::KIND);
    match event.apply_for(outcome, instance) {
        Applied::Unchanged | Applied::Set => {}
        Applied::Fallback(error) => {
            if let Some(suppressed) = instance.admit_warning() {
                tracing::warn!(plugin, event = event_name, %error, suppressed,
                    "wasm plugin set a result with an invalid text component; the result applies with a fallback text");
            }
        }
        Applied::Degraded(reason) => {
            if let Some(suppressed) = instance.admit_warning() {
                tracing::warn!(plugin, event = event_name, %reason, suppressed,
                    "wasm plugin set a result the host cannot use as given; a safe fallback applies instead");
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

macro_rules! event_kinds {
    ($($kind:ident => $name:literal,)*) => {
        pub(crate) const fn kind_name(kind: EventKind) -> &'static str {
            match kind {
                $(EventKind::$kind => $name,)*
            }
        }

        #[cfg(test)]
        const ALL_KINDS: &[EventKind] = &[$(EventKind::$kind,)*];
    };
}

event_kinds! {
    PreLogin => "pre-login",
    PostLogin => "post-login",
    Disconnect => "disconnect",
    OnlineAuthFailed => "online-auth-failed",
    PermissionsSetup => "permissions-setup",
    PlayerChooseInitialServer => "player-choose-initial-server",
    ServerPreConnect => "server-pre-connect",
    ServerConnected => "server-connected",
    ServerPostConnect => "server-post-connect",
    KickedFromServer => "kicked-from-server",
    ChatMessage => "chat-message",
    ProxyPing => "proxy-ping",
    ProxyInitialize => "proxy-initialize",
    ProxyShutdown => "proxy-shutdown",
    ConfigReload => "config-reload",
    ServerStateChange => "server-state-change",
    BackendHealth => "backend-health",
    Login => "login",
    GameProfileRequest => "game-profile-request",
    CommandExecute => "command-execute",
    ConnectionHandshake => "connection-handshake",
    ConnectionRejected => "connection-rejected",
    LimboEnter => "limbo-enter",
    LimboExit => "limbo-exit",
    PlayerClientBrand => "player-client-brand",
    PlayerSettingsChanged => "player-settings-changed",
    PlayerChannelRegister => "player-channel-register",
    PluginMessage => "plugin-message",
    BanIssued => "ban-issued",
    BanRevoked => "ban-revoked",
    PluginEnabled => "plugin-enabled",
    PluginDisabled => "plugin-disabled",
    PreTransfer => "pre-transfer",
    PlayerResourcePackStatus => "player-resource-pack-status",
    NamedEvent => "named-event",
    RawPacket => "raw-packet",
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
pub(crate) fn steve() -> Arc<dyn infrarust_api::player::Player> {
    let (player, _commands) = infrarust_core::player::PlayerSession::new_test(true);
    player
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use infrarust_core::event_bus::EventBusImpl;
    use infrarust_core::plugin::tracking::TrackingEventBus;

    use super::*;

    #[test]
    fn every_event_kind_registers_one_listener() {
        let bus = TrackingEventBus::new(Arc::new(EventBusImpl::new()), "guest");
        let kinds = ALL_KINDS
            .iter()
            .copied()
            .filter(|kind| *kind != EventKind::RawPacket);
        for (at, kind) in kinds.enumerate() {
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
