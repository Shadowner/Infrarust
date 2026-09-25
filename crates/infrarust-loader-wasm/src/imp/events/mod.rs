mod chat;
mod connection;
mod login;
mod proxy;

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{Event, EventPriority, ListenerHandle};
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::connection::{
    KickedFromServerEvent, PlayerChooseInitialServerEvent, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent, PostLoginEvent, PreLoginEvent,
};
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
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

pub(crate) trait WasmEvent: Event {
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

pub(crate) fn register(
    bus: &dyn EventBus,
    instance: InstanceRef,
    kind: EventKind,
    priority: EventPriority,
    listener: u64,
) -> ListenerHandle {
    match kind {
        EventKind::PreLogin => subscribe::<PreLoginEvent>(bus, instance, priority, listener),
        EventKind::PostLogin => subscribe::<PostLoginEvent>(bus, instance, priority, listener),
        EventKind::Disconnect => subscribe::<DisconnectEvent>(bus, instance, priority, listener),
        EventKind::OnlineAuthFailed => {
            subscribe::<OnlineAuthFailed>(bus, instance, priority, listener)
        }
        EventKind::PermissionsSetup => {
            subscribe::<PermissionsSetupEvent>(bus, instance, priority, listener)
        }
        EventKind::PlayerChooseInitialServer => {
            subscribe::<PlayerChooseInitialServerEvent>(bus, instance, priority, listener)
        }
        EventKind::ServerPreConnect => {
            subscribe::<ServerPreConnectEvent>(bus, instance, priority, listener)
        }
        EventKind::ServerConnected => {
            subscribe::<ServerConnectedEvent>(bus, instance, priority, listener)
        }
        EventKind::ServerPostConnect => {
            subscribe::<ServerPostConnectEvent>(bus, instance, priority, listener)
        }
        EventKind::KickedFromServer => {
            subscribe::<KickedFromServerEvent>(bus, instance, priority, listener)
        }
        EventKind::ChatMessage => subscribe::<ChatMessageEvent>(bus, instance, priority, listener),
        EventKind::ProxyPing => subscribe::<ProxyPingEvent>(bus, instance, priority, listener),
        EventKind::ProxyInitialize => {
            subscribe::<ProxyInitializeEvent>(bus, instance, priority, listener)
        }
        EventKind::ProxyShutdown => {
            subscribe::<ProxyShutdownEvent>(bus, instance, priority, listener)
        }
        EventKind::ConfigReload => {
            subscribe::<ConfigReloadEvent>(bus, instance, priority, listener)
        }
        EventKind::ServerStateChange => {
            subscribe::<ServerStateChangeEvent>(bus, instance, priority, listener)
        }
        EventKind::BackendHealth => {
            subscribe::<BackendHealthEvent>(bus, instance, priority, listener)
        }
    }
}

fn subscribe<E: WasmEvent>(
    bus: &dyn EventBus,
    instance: InstanceRef,
    priority: EventPriority,
    listener: u64,
) -> ListenerHandle {
    bus.subscribe_async::<E, _>(priority, move |event: &mut E| {
        let wit = event.to_wit();
        let instance = instance.clone();
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
    })
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

    const ALL_KINDS: [EventKind; 17] = [
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
    ];

    #[test]
    fn every_event_kind_registers_one_listener() {
        let bus = TrackingEventBus::new(Arc::new(EventBusImpl::new()), "guest");
        for (at, kind) in ALL_KINDS.into_iter().enumerate() {
            register(
                &bus,
                InstanceRef::detached(),
                kind,
                EventPriority::NORMAL,
                1,
            );
            assert_eq!(bus.tracked_count(), at + 1, "{}", kind_name(kind));
        }
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
