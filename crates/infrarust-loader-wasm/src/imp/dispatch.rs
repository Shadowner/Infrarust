//! Event dispatch: marshals native events into the unified guest `handle-event`
//! export and applies the returned outcome back onto the (6 modifiable) events.

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{EventPriority, ListenerHandle, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::connection::{
    KickCause, KickedFromServerEvent, KickedFromServerResult, PlayerChooseInitialServerEvent,
    PlayerChooseInitialServerResult, ServerConnectedEvent, ServerPostConnectEvent,
    ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent, PostLoginEvent, PreLoginEvent,
    PreLoginResult,
};
use infrarust_api::events::proxy::{
    ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent, ProxyShutdownEvent,
    ServerStateChangeEvent,
};
use infrarust_api::types::{Component, ProtocolVersion, ServerId};

use crate::actor::InstanceRef;

use crate::bindings::exports::infrarust::plugin::guest as wg;
use crate::bindings::infrarust::plugin::event_bus::EventKind;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::plugin::call_guest;

pub(crate) fn priority_from_wit(p: wt::EventPriority) -> EventPriority {
    EventPriority::custom(p)
}

pub(crate) fn register_event_handler(
    bus: &dyn EventBus,
    instance: InstanceRef,
    kind: EventKind,
    priority: EventPriority,
    listener_id: u64,
) -> Option<ListenerHandle> {
    let handle = match kind {
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
            bus.subscribe_async::<ServerPostConnectEvent, _>(priority, move |ev| {
                let wit = ev_server_switch(ev);
                let instance = instance.clone();
                Box::pin(async move {
                    if let Some(wit) = wit {
                        dispatch(instance, listener_id, wit, |_| {}).await;
                    }
                })
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
        EventKind::RawPacket => return None,
    };
    Some(handle)
}

async fn dispatch<F: FnOnce(wg::EventOutcome)>(
    instance: InstanceRef,
    listener_id: u64,
    wit: wg::Event,
    apply: F,
) {
    let outcome = call_guest(instance, "handle-event", move |store, bindings| {
        Box::pin(async move {
            bindings
                .infrarust_plugin_guest()
                .call_handle_event(&mut *store, listener_id, &wit)
                .await
        })
    })
    .await;
    if let Some(outcome) = outcome {
        apply(outcome);
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
        player_id: e.player_id().as_u64(),
        protocol_version: e.protocol_version.raw(),
    })
}

fn ev_disconnect(e: &DisconnectEvent) -> wg::Event {
    wg::Event::Disconnect(wg::DisconnectEvent {
        player_id: e.player_id().as_u64(),
        username: e.username().to_string(),
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
        player_id: e.player_id().as_u64(),
        profile: convert::game_profile_to_wit(e.profile()),
        online_mode: e.online_mode,
    })
}

fn ev_server_pre_connect(e: &ServerPreConnectEvent) -> wg::Event {
    wg::Event::ServerPreConnect(wg::ServerPreConnectEvent {
        player_id: e.player_id().as_u64(),
        profile: convert::game_profile_to_wit(e.profile()),
        original_server: e.server.as_str().to_string(),
    })
}

fn ev_server_connected(e: &ServerConnectedEvent) -> wg::Event {
    wg::Event::ServerConnected(wg::ServerConnectedEvent {
        player_id: e.player_id().as_u64(),
        server: e.server.as_str().to_string(),
    })
}

fn ev_server_switch(e: &ServerPostConnectEvent) -> Option<wg::Event> {
    let previous = e.switched_from()?;
    Some(wg::Event::ServerSwitch(wg::ServerSwitchEvent {
        player_id: e.player_id().as_u64(),
        previous_server: previous.as_str().to_string(),
        new_server: e.server.as_str().to_string(),
    }))
}

fn ev_kicked_from_server(e: &KickedFromServerEvent) -> wg::Event {
    let reason = match (&e.reason, &e.cause) {
        (Some(reason), _) => reason.clone(),
        (None, KickCause::Unreachable { error }) => Component::text(error),
        (None, _) => Component::text(""),
    };
    wg::Event::KickedFromServer(wg::KickedFromServerEvent {
        player_id: e.player_id().as_u64(),
        server: e.server.as_str().to_string(),
        reason: convert::component_to_wit(&reason),
    })
}

fn ev_player_choose_initial_server(e: &PlayerChooseInitialServerEvent) -> wg::Event {
    wg::Event::PlayerChooseInitialServer(wg::PlayerChooseInitialServerEvent {
        player_id: e.player_id().as_u64(),
        profile: convert::game_profile_to_wit(e.profile()),
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
        player_id: e.player_id().as_u64(),
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
                    reason: Some(convert::component_from_wit(&c)),
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
                reason: Some(convert::component_from_wit(&c)),
            },
            wg::ChatMessageResult::Modify(message) => ChatMessageResult::Modify { message },
        };
        ev.set_result(native);
    }
}

fn apply_proxy_ping(outcome: wg::EventOutcome, ev: &mut ProxyPingEvent) {
    if let wg::EventOutcome::ProxyPing(r) = outcome {
        let response = &mut ev.response;
        response.description = convert::component_from_wit(&r.description);
        response.max_players = r.max_players;
        response.online_players = r.online_players;
        response.protocol_version = ProtocolVersion::new(r.protocol_version);
        response.version_name = r.version_name;
        response.favicon = r.favicon;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use infrarust_core::event_bus::EventBusImpl;
    use infrarust_core::plugin::tracking::TrackingEventBus;

    use super::*;

    fn guest_bus() -> TrackingEventBus {
        TrackingEventBus::new(Arc::new(EventBusImpl::new()), "guest")
    }

    #[test]
    fn a_raw_packet_subscription_registers_no_listener() {
        let bus = guest_bus();

        let handle = register_event_handler(
            &bus,
            InstanceRef::detached(),
            EventKind::RawPacket,
            EventPriority::NORMAL,
            1,
        );

        assert_eq!(handle, None);
        assert_eq!(bus.tracked_count(), 0);
    }

    fn post_connect(server: &str, previous: Option<&str>) -> ServerPostConnectEvent {
        let (player, _commands) = infrarust_core::player::PlayerSession::new_test(true);
        ServerPostConnectEvent::new(
            Arc::new(player),
            ServerId::new(server),
            previous.map(ServerId::new),
        )
    }

    #[test]
    fn only_a_join_from_another_server_is_a_server_switch() {
        assert!(ev_server_switch(&post_connect("lobby", None)).is_none());
        assert!(ev_server_switch(&post_connect("lobby", Some("lobby"))).is_none());

        let Some(wg::Event::ServerSwitch(record)) =
            ev_server_switch(&post_connect("survival", Some("lobby")))
        else {
            panic!("a join from another server must reach server-switch listeners");
        };
        assert_eq!(record.player_id, 1);
        assert_eq!(record.previous_server, "lobby");
        assert_eq!(record.new_server, "survival");
    }

    #[test]
    fn other_event_kinds_still_register_a_listener() {
        let bus = guest_bus();

        let handle = register_event_handler(
            &bus,
            InstanceRef::detached(),
            EventKind::ConfigReload,
            EventPriority::NORMAL,
            1,
        );

        assert!(handle.is_some());
        assert_eq!(bus.tracked_count(), 1);
    }

    #[test]
    fn a_ping_outcome_keeps_the_player_sample() {
        let notch = ("Notch".to_string(), uuid::Uuid::from_u128(7));
        let mut response = PingResponse::new(
            Component::text("motd"),
            20,
            1,
            ProtocolVersion::new(774),
            "Infrarust".into(),
            None,
        );
        response.player_sample = vec![notch.clone()];
        let mut event = ProxyPingEvent::new(
            "127.0.0.1:25565".parse().unwrap(),
            Some(ServerId::new("lobby")),
            Some("lobby.test".into()),
            ProtocolVersion::new(774),
            false,
            response,
        );
        let mut outcome = ping_response_to_wit(&event.response);
        outcome.max_players = 99;

        apply_proxy_ping(wg::EventOutcome::ProxyPing(outcome), &mut event);

        assert_eq!(event.response.max_players, 99);
        assert_eq!(event.response.player_sample, [notch]);
    }
}
