//! Event dispatch: marshals native events into the unified guest `handle-event`
//! export and applies the returned outcome back onto the (6 modifiable) events.

use std::sync::Weak;

use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::{EventPriority, ListenerHandle, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::connection::{
    KickedFromServerEvent, KickedFromServerResult, PlayerChooseInitialServerEvent,
    PlayerChooseInitialServerResult, ServerConnectedEvent, ServerPreConnectEvent,
    ServerPreConnectResult, ServerSwitchEvent,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent, PostLoginEvent, PreLoginEvent,
    PreLoginResult,
};
use infrarust_api::events::packet::RawPacketEvent;
use infrarust_api::events::proxy::{
    ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent, ProxyShutdownEvent,
    ServerStateChangeEvent,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::types::{ProtocolVersion, ServerId};
use tokio::sync::Mutex;

use crate::bindings::exports::infrarust::plugin::guest as wg;
use crate::bindings::infrarust::plugin::event_bus::EventKind;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::plugin::WasmInstance;

pub(crate) fn priority_from_wit(p: wt::EventPriority) -> EventPriority {
    EventPriority::custom(p)
}

pub(crate) fn register_event_handler(
    ctx: &dyn PluginContext,
    instance: Weak<Mutex<WasmInstance>>,
    kind: EventKind,
    priority: EventPriority,
    listener_id: u64,
) -> ListenerHandle {
    let bus = ctx.event_bus();
    match kind {
        EventKind::PreLogin => bus.subscribe_async::<PreLoginEvent, _>(priority, move |ev| {
            let wit = ev_pre_login(ev);
            Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                apply_pre_login(o, ev)
            }))
        }),
        EventKind::PostLogin => bus.subscribe_async::<PostLoginEvent, _>(priority, move |ev| {
            let wit = ev_post_login(ev);
            Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
        }),
        EventKind::Disconnect => bus.subscribe_async::<DisconnectEvent, _>(priority, move |ev| {
            let wit = ev_disconnect(ev);
            Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
        }),
        EventKind::OnlineAuthFailed => {
            bus.subscribe_async::<OnlineAuthFailed, _>(priority, move |ev| {
                let wit = ev_online_auth_failed(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
            })
        }
        EventKind::PermissionsSetup => {
            bus.subscribe_async::<PermissionsSetupEvent, _>(priority, move |ev| {
                let wit = ev_permissions_setup(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
            })
        }
        EventKind::ServerPreConnect => {
            bus.subscribe_async::<ServerPreConnectEvent, _>(priority, move |ev| {
                let wit = ev_server_pre_connect(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                    apply_server_pre_connect(o, ev);
                }))
            })
        }
        EventKind::ServerConnected => {
            bus.subscribe_async::<ServerConnectedEvent, _>(priority, move |ev| {
                let wit = ev_server_connected(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
            })
        }
        EventKind::ServerSwitch => {
            bus.subscribe_async::<ServerSwitchEvent, _>(priority, move |ev| {
                let wit = ev_server_switch(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
            })
        }
        EventKind::KickedFromServer => {
            bus.subscribe_async::<KickedFromServerEvent, _>(priority, move |ev| {
                let wit = ev_kicked_from_server(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                    apply_kicked_from_server(o, ev);
                }))
            })
        }
        EventKind::PlayerChooseInitialServer => bus
            .subscribe_async::<PlayerChooseInitialServerEvent, _>(priority, move |ev| {
                let wit = ev_player_choose_initial_server(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                    apply_player_choose_initial_server(o, ev);
                }))
            }),
        EventKind::ProxyPing => bus.subscribe_async::<ProxyPingEvent, _>(priority, move |ev| {
            let wit = ev_proxy_ping(ev);
            Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                apply_proxy_ping(o, ev)
            }))
        }),
        EventKind::ProxyInitialize => {
            bus.subscribe_async::<ProxyInitializeEvent, _>(priority, move |_ev| {
                Box::pin(dispatch(
                    instance.clone(),
                    listener_id,
                    wg::Event::ProxyInitialize,
                    |_| {},
                ))
            })
        }
        EventKind::ProxyShutdown => {
            bus.subscribe_async::<ProxyShutdownEvent, _>(priority, move |_ev| {
                Box::pin(dispatch(
                    instance.clone(),
                    listener_id,
                    wg::Event::ProxyShutdown,
                    |_| {},
                ))
            })
        }
        EventKind::ConfigReload => {
            bus.subscribe_async::<ConfigReloadEvent, _>(priority, move |_ev| {
                Box::pin(dispatch(
                    instance.clone(),
                    listener_id,
                    wg::Event::ConfigReload,
                    |_| {},
                ))
            })
        }
        EventKind::ServerStateChange => {
            bus.subscribe_async::<ServerStateChangeEvent, _>(priority, move |ev| {
                let wit = ev_server_state_change(ev);
                Box::pin(dispatch(instance.clone(), listener_id, wit, |_| {}))
            })
        }
        EventKind::ChatMessage => bus.subscribe_async::<ChatMessageEvent, _>(priority, move |ev| {
            let wit = ev_chat_message(ev);
            Box::pin(dispatch(instance.clone(), listener_id, wit, move |o| {
                apply_chat_message(o, ev)
            }))
        }),
        EventKind::RawPacket => {
            bus.subscribe_async::<RawPacketEvent, _>(priority, move |_ev| Box::pin(async {}))
        }
    }
}

async fn dispatch<F: FnOnce(wg::EventOutcome)>(
    instance: Weak<Mutex<WasmInstance>>,
    listener_id: u64,
    wit: wg::Event,
    apply: F,
) {
    let Some(arc) = instance.upgrade() else {
        return;
    };
    let mut guard = arc.lock().await;
    let WasmInstance { store, bindings } = &mut *guard;
    if store.data().is_poisoned() {
        return;
    }
    store.data_mut().reset_epoch_budget();
    match bindings
        .infrarust_plugin_guest()
        .call_handle_event(&mut *store, listener_id, &wit)
        .await
    {
        Ok(outcome) => apply(outcome),
        Err(trap) => {
            tracing::error!(plugin = %store.data().plugin_id, error = %trap,
                "wasm guest trapped in handle-event; poisoning instance");
            store.data_mut().set_poisoned();
        }
    }
}

fn ev_pre_login(e: &PreLoginEvent) -> wg::Event {
    wg::Event::PreLogin(wg::PreLoginEvent {
        profile: convert::game_profile_to_wit(&e.profile),
        remote_addr: e.remote_addr.to_string(),
        protocol_version: e.protocol_version.raw(),
        server_domain: e.server_domain.clone(),
    })
}

fn ev_post_login(e: &PostLoginEvent) -> wg::Event {
    wg::Event::PostLogin(wg::PostLoginEvent {
        profile: convert::game_profile_to_wit(&e.profile),
        player_id: e.player_id.as_u64(),
        protocol_version: e.protocol_version.raw(),
    })
}

fn ev_disconnect(e: &DisconnectEvent) -> wg::Event {
    wg::Event::Disconnect(wg::DisconnectEvent {
        player_id: e.player_id.as_u64(),
        username: e.username.clone(),
        last_server: e.last_server.as_ref().map(|s| s.as_str().to_string()),
    })
}

fn ev_online_auth_failed(e: &OnlineAuthFailed) -> wg::Event {
    wg::Event::OnlineAuthFailed(wg::OnlineAuthFailedEvent {
        username: e.username.clone(),
    })
}

fn ev_permissions_setup(e: &PermissionsSetupEvent) -> wg::Event {
    wg::Event::PermissionsSetup(wg::PermissionsSetupEvent {
        player_id: e.player_id.as_u64(),
        profile: convert::game_profile_to_wit(&e.profile),
        online_mode: e.online_mode,
    })
}

fn ev_server_pre_connect(e: &ServerPreConnectEvent) -> wg::Event {
    wg::Event::ServerPreConnect(wg::ServerPreConnectEvent {
        player_id: e.player_id.as_u64(),
        profile: convert::game_profile_to_wit(&e.profile),
        original_server: e.original_server.as_str().to_string(),
    })
}

fn ev_server_connected(e: &ServerConnectedEvent) -> wg::Event {
    wg::Event::ServerConnected(wg::ServerConnectedEvent {
        player_id: e.player_id.as_u64(),
        server: e.server.as_str().to_string(),
    })
}

fn ev_server_switch(e: &ServerSwitchEvent) -> wg::Event {
    wg::Event::ServerSwitch(wg::ServerSwitchEvent {
        player_id: e.player_id.as_u64(),
        previous_server: e.previous_server.as_str().to_string(),
        new_server: e.new_server.as_str().to_string(),
    })
}

fn ev_kicked_from_server(e: &KickedFromServerEvent) -> wg::Event {
    wg::Event::KickedFromServer(wg::KickedFromServerEvent {
        player_id: e.player_id.as_u64(),
        server: e.server.as_str().to_string(),
        reason: convert::component_to_wit(&e.reason),
    })
}

fn ev_player_choose_initial_server(e: &PlayerChooseInitialServerEvent) -> wg::Event {
    wg::Event::PlayerChooseInitialServer(wg::PlayerChooseInitialServerEvent {
        player_id: e.player_id.as_u64(),
        profile: convert::game_profile_to_wit(&e.profile),
        initial_server: e.initial_server.as_str().to_string(),
    })
}

fn ev_proxy_ping(e: &ProxyPingEvent) -> wg::Event {
    wg::Event::ProxyPing(wg::ProxyPingEvent {
        remote_addr: e.remote_addr.to_string(),
        response: ping_response_to_wit(&e.response),
    })
}

fn ev_server_state_change(e: &ServerStateChangeEvent) -> wg::Event {
    wg::Event::ServerStateChange(wg::ServerStateChangeEvent {
        server: e.server.as_str().to_string(),
        old_state: convert::server_state_to_wit(e.old_state),
        new_state: convert::server_state_to_wit(e.new_state),
    })
}

fn ev_chat_message(e: &ChatMessageEvent) -> wg::Event {
    wg::Event::ChatMessage(wg::ChatMessageEvent {
        player_id: e.player_id.as_u64(),
        message: e.message.clone(),
    })
}

fn ping_response_to_wit(r: &PingResponse) -> wg::PingResponse {
    wg::PingResponse {
        description: convert::component_to_wit(&r.description),
        max_players: r.max_players,
        online_players: r.online_players,
        protocol_version: r.protocol_version.raw(),
        version_name: r.version_name.clone(),
        favicon: r.favicon.clone(),
    }
}

fn apply_pre_login(outcome: wg::EventOutcome, ev: &mut PreLoginEvent) {
    if let wg::EventOutcome::PreLogin(r) = outcome {
        let native = match r {
            wg::PreLoginResult::Allowed => PreLoginResult::Allowed,
            wg::PreLoginResult::Denied(c) => PreLoginResult::Denied {
                reason: convert::component_from_wit(&c),
            },
            wg::PreLoginResult::ForceOffline => PreLoginResult::ForceOffline,
            wg::PreLoginResult::ForceOnline => PreLoginResult::ForceOnline,
        };
        ev.set_result(native);
    }
}

fn apply_server_pre_connect(outcome: wg::EventOutcome, ev: &mut ServerPreConnectEvent) {
    if let wg::EventOutcome::ServerPreConnect(r) = outcome {
        let native = match r {
            wg::ServerPreConnectResult::Allowed => ServerPreConnectResult::Allowed,
            wg::ServerPreConnectResult::ConnectTo(s) => {
                ServerPreConnectResult::ConnectTo(ServerId::from(s))
            }
            wg::ServerPreConnectResult::SendToLimbo(h) => {
                ServerPreConnectResult::SendToLimbo { limbo_handlers: h }
            }
            wg::ServerPreConnectResult::Denied(c) => ServerPreConnectResult::Denied {
                reason: convert::component_from_wit(&c),
            },
        };
        ev.set_result(native);
    }
}

fn apply_kicked_from_server(outcome: wg::EventOutcome, ev: &mut KickedFromServerEvent) {
    if let wg::EventOutcome::KickedFromServer(r) = outcome {
        let native = match r {
            wg::KickedFromServerResult::DisconnectPlayer(c) => {
                KickedFromServerResult::DisconnectPlayer {
                    reason: convert::component_from_wit(&c),
                }
            }
            wg::KickedFromServerResult::RedirectTo(s) => {
                KickedFromServerResult::RedirectTo(ServerId::from(s))
            }
            wg::KickedFromServerResult::SendToLimbo(h) => {
                KickedFromServerResult::SendToLimbo { limbo_handlers: h }
            }
            wg::KickedFromServerResult::Notify(c) => KickedFromServerResult::Notify {
                message: convert::component_from_wit(&c),
            },
        };
        ev.set_result(native);
    }
}

fn apply_player_choose_initial_server(
    outcome: wg::EventOutcome,
    ev: &mut PlayerChooseInitialServerEvent,
) {
    if let wg::EventOutcome::PlayerChooseInitialServer(r) = outcome {
        let native = match r {
            wg::PlayerChooseInitialServerResult::Allowed => {
                PlayerChooseInitialServerResult::Allowed
            }
            wg::PlayerChooseInitialServerResult::Redirect(s) => {
                PlayerChooseInitialServerResult::Redirect(ServerId::from(s))
            }
            wg::PlayerChooseInitialServerResult::SendToLimbo(h) => {
                PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers: h }
            }
        };
        ev.set_result(native);
    }
}

fn apply_chat_message(outcome: wg::EventOutcome, ev: &mut ChatMessageEvent) {
    if let wg::EventOutcome::ChatMessage(r) = outcome {
        let native = match r {
            wg::ChatMessageResult::Allow => ChatMessageResult::Allow,
            wg::ChatMessageResult::Deny(c) => ChatMessageResult::Deny {
                reason: convert::component_from_wit(&c),
            },
            wg::ChatMessageResult::Modify(m) => ChatMessageResult::Modify { new_message: m },
        };
        ev.set_result(native);
    }
}

fn apply_proxy_ping(outcome: wg::EventOutcome, ev: &mut ProxyPingEvent) {
    if let wg::EventOutcome::ProxyPing(r) = outcome {
        ev.response = PingResponse::new(
            convert::component_from_wit(&r.description),
            r.max_players,
            r.online_players,
            ProtocolVersion::new(r.protocol_version),
            r.version_name,
            r.favicon,
        );
    }
}
