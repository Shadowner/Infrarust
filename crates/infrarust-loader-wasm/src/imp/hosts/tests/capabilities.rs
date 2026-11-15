use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_plugin_common::capability::gates::{GATES, SUBSCRIBE_GATES};

use crate::bindings::infrarust::plugin::events::EventKind;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, connection_claims, event_bus,
    limbo, load_balancer, messaging, permissions, players, providers, scheduler, server_manager,
    types as wt,
};
use crate::config::SandboxLimits;
use crate::store_state::build_probe_state;

use super::event_bus::packet_filter;
use super::players::boss_bar;
use super::players::resource_pack;
use super::{address, channel, command_spec, is_denied, nobody, text_of};

#[tokio::test]
async fn services_are_unavailable_while_the_plugin_is_inspected() {
    let mut state = build_probe_state("probe".to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::native_trusted());
    let unavailable = config_service::Host::get_value(&mut state, "greeting".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(unavailable.kind, wt::ErrorKind::Unavailable);
    assert!(
        players::Host::get(&mut state, 1).await.unwrap().is_none(),
        "an infallible read answers neutrally"
    );
}

macro_rules! expect {
    ($failures:ident, $capability:expr, $call:literal, $result:expr, $pattern:pat $(if $guard:expr)?) => {{
        let result = $result;
        if !matches!(result, $pattern $(if $guard)?) {
            $failures.push(format!("{:?} {}: {:?}", $capability, $call, result));
        }
    }};
}

fn codec_metadata() -> codec_registry::CodecFilterMetadata {
    codec_registry::CodecFilterMetadata {
        id: "ops".to_owned(),
        priority: codec_registry::FilterPriority::Normal,
        after: vec![],
        before: vec![],
        required: false,
    }
}

async fn unrefused_calls(capability: Capability) -> Vec<String> {
    let mut state = build_probe_state("denied".to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::native_trusted().without(capability));
    let mut failures = Vec::new();
    let s = &mut state;
    macro_rules! denied {
        ($call:literal, $result:expr) => {{
            let result = $result.expect("a refusal is a host-error, not a trap");
            if !is_denied(&result, capability) {
                failures.push(format!("{capability:?} {}: {result:?}", $call));
            }
        }};
    }
    match capability {
        Capability::Ban => {
            let request = ban_service::BanRequest {
                target: nobody(),
                reason: None,
                duration_ms: None,
                kick: true,
                silent: false,
            };
            denied!("ban", ban_service::Host::ban(s, request).await);
            denied!("unban", ban_service::Host::unban(s, nobody()).await);
            denied!("get", ban_service::Host::get(s, nobody()).await);
            denied!("list", ban_service::Host::list(s, None, 10).await);
        }
        Capability::ServerManage => {
            denied!(
                "set-drained",
                load_balancer::Host::set_drained(s, "lobby".into(), address(), true).await
            );
            denied!(
                "reset-backend",
                load_balancer::Host::reset_backend(s, "lobby".into(), address()).await
            );
            denied!(
                "start",
                server_manager::Host::start(s, "lobby".into()).await
            );
            denied!("stop", server_manager::Host::stop(s, "lobby".into()).await);
            denied!(
                "get-state",
                server_manager::Host::get_state(s, "lobby".into()).await
            );
            denied!("list", server_manager::Host::list(s).await);
        }
        Capability::ConfigRead => {
            denied!(
                "get-server",
                config_service::Host::get_server(s, "lobby".into()).await
            );
            denied!(
                "get-server-by-domain",
                config_service::Host::get_server_by_domain(s, "play.example.com".into()).await
            );
            denied!("list-servers", config_service::Host::list_servers(s).await);
            denied!(
                "get-value",
                config_service::Host::get_value(s, "greeting".into()).await
            );
            denied!(
                "get-server-document",
                config_service::Host::get_server_document(s, "lobby".into()).await
            );
            denied!(
                "list-server-sources",
                config_service::Host::list_server_sources(s).await
            );
            denied!(
                "get-proxy-config-document",
                config_service::Host::get_proxy_config_document(s).await
            );
            denied!(
                "get-effective-proxy-config-document",
                config_service::Host::get_effective_proxy_config_document(s).await
            );
            denied!(
                "strategy",
                load_balancer::Host::strategy(s, "lobby".into()).await
            );
            denied!(
                "backends",
                load_balancer::Host::backends(s, "lobby".into()).await
            );
        }
        Capability::ConfigWrite => {
            denied!(
                "write-proxy-config-document",
                config_service::Host::write_proxy_config_document(s, String::new()).await
            );
        }
        Capability::PluginMessaging => {
            denied!(
                "register-channel",
                messaging::Host::register_channel(s, channel()).await
            );
            denied!(
                "unregister-channel",
                messaging::Host::unregister_channel(s, channel()).await
            );
            denied!("channels", messaging::Host::channels(s).await);
            denied!(
                "send-to-player",
                messaging::Host::send_to_player(s, 1, channel(), vec![1]).await
            );
            denied!(
                "send-to-backend",
                messaging::Host::send_to_backend(s, 1, channel(), vec![1]).await
            );
            denied!(
                "send-to-server",
                messaging::Host::send_to_server(s, "lobby".into(), channel(), vec![1]).await
            );
            denied!(
                "subscribe(plugin-message)",
                event_bus::Host::subscribe(s, EventKind::PluginMessage, 128).await
            );
            denied!(
                "subscribe(login-plugin-request)",
                event_bus::Host::subscribe(s, EventKind::LoginPluginRequest, 128).await
            );
            denied!(
                "subscribe(login-plugin-response)",
                event_bus::Host::subscribe(s, EventKind::LoginPluginResponse, 128).await
            );
        }
        Capability::ConnectionHandoff => {
            denied!(
                "register-prefix",
                connection_claims::Host::register_prefix(s, b"XFER".to_vec()).await
            );
            denied!(
                "unregister-prefix",
                connection_claims::Host::unregister_prefix(s, b"XFER".to_vec()).await
            );
            denied!(
                "subscribe(raw-connection)",
                event_bus::Host::subscribe(s, EventKind::RawConnection, 128).await
            );
            denied!(
                "subscribe(login-claim)",
                event_bus::Host::subscribe(s, EventKind::LoginClaim, 128).await
            );
        }
        Capability::PlayerRead => {
            expect!(
                failures,
                capability,
                "get",
                players::Host::get(s, 1).await,
                Ok(None)
            );
            expect!(
                failures,
                capability,
                "get-by-name",
                players::Host::get_by_name(s, "Steve".into()).await,
                Ok(None)
            );
            expect!(
                failures,
                capability,
                "get-by-uuid",
                players::Host::get_by_uuid(s, wt::Uuid { hi: 0, lo: 0 }).await,
                Ok(None)
            );
            expect!(failures, capability, "list",
                players::Host::list(s, None).await, Ok(ref v) if v.is_empty());
            expect!(failures, capability, "get-by-ip",
                players::Host::get_by_ip(s, wt::IpAddress::Ipv4((127, 0, 0, 1))).await,
                Ok(ref v) if v.is_empty());
            expect!(
                failures,
                capability,
                "count",
                players::Host::count(s, None).await,
                Ok(0)
            );
            denied!(
                "has-permission",
                players::Host::has_permission(s, 1, "x".into()).await
            );
        }
        Capability::PlayerWrite => {
            denied!(
                "send-message",
                players::Host::send_message(s, 1, text_of("hi")).await
            );
            denied!(
                "send-action-bar",
                players::Host::send_action_bar(s, 1, text_of("hi")).await
            );
            let title = wt::TitleData {
                title: text_of("t"),
                subtitle: text_of("s"),
                fade_in_ticks: 0,
                stay_ticks: 0,
                fade_out_ticks: 0,
            };
            denied!("send-title", players::Host::send_title(s, 1, title).await);
            denied!(
                "switch-server",
                players::Host::switch_server(s, 1, "lobby".into()).await
            );
            denied!(
                "disconnect",
                players::Host::disconnect(s, 1, text_of("bye")).await
            );
            denied!(
                "connect",
                players::Host::connect(s, 1, "lobby".into()).await
            );
            denied!(
                "set-player-list-header-footer",
                players::Host::set_player_list_header_footer(s, 1, text_of("h"), text_of("f"))
                    .await
            );
            denied!("clear-title", players::Host::clear_title(s, 1, true).await);
            denied!(
                "show-boss-bar",
                players::Host::show_boss_bar(s, 1, boss_bar()).await
            );
            denied!(
                "update-boss-bar",
                players::Host::update_boss_bar(
                    s,
                    wt::Uuid { hi: 0, lo: 1 },
                    players::BossBarUpdate::Progress(0.5)
                )
                .await
            );
            denied!(
                "hide-boss-bar",
                players::Host::hide_boss_bar(s, wt::Uuid { hi: 0, lo: 1 }).await
            );
            denied!(
                "send-resource-pack",
                players::Host::send_resource_pack(s, 1, resource_pack()).await
            );
            denied!(
                "remove-resource-pack",
                players::Host::remove_resource_pack(s, 1, None).await
            );
            denied!("transfer", players::Host::transfer(s, 1, address()).await);
            denied!(
                "store-cookie",
                players::Host::store_cookie(s, 1, "k".into(), vec![1]).await
            );
            denied!(
                "request-cookie",
                players::Host::request_cookie(s, 1, "k".into()).await
            );
            denied!(
                "refresh-permissions",
                players::Host::refresh_permissions(s, 1).await
            );
        }
        Capability::RawPacket => {
            let packet = wt::RawPacket {
                packet_id: 1,
                data: vec![],
            };
            denied!(
                "send-packet",
                players::Host::send_packet(s, 1, packet).await
            );
            denied!(
                "subscribe-packets",
                event_bus::Host::subscribe_packets(s, vec![packet_filter()], 128).await
            );
            denied!(
                "subscribe(raw-packet)",
                event_bus::Host::subscribe(s, EventKind::RawPacket, 128).await
            );
        }
        Capability::ChatIntercept => {
            denied!(
                "subscribe(chat-message)",
                event_bus::Host::subscribe(s, EventKind::ChatMessage, 128).await
            );
            denied!(
                "subscribe(command-execute)",
                event_bus::Host::subscribe(s, EventKind::CommandExecute, 128).await
            );
        }
        Capability::EventBus => {
            denied!(
                "subscribe",
                event_bus::Host::subscribe(s, EventKind::PostLogin, 128).await
            );
            denied!("unsubscribe", event_bus::Host::unsubscribe(s, 1).await);
            denied!(
                "subscribe-named",
                event_bus::Host::subscribe_named(s, "x".into(), 128).await
            );
            denied!(
                "fire-named",
                event_bus::Host::fire_named(s, "x".into(), "text/plain".into(), vec![]).await
            );
            denied!(
                "subscribe-packets",
                event_bus::Host::subscribe_packets(s, vec![packet_filter()], 128).await
            );
        }
        Capability::Command => {
            denied!(
                "register",
                command_manager::Host::register(s, command_spec(), 1).await
            );
            denied!(
                "unregister",
                command_manager::Host::unregister(s, "probe".into()).await
            );
            denied!("get", command_manager::Host::get(s, "probe".into()).await);
            denied!(
                "get-by-name",
                command_manager::Host::get_by_name(s, "probe".into()).await
            );
            denied!(
                "get-by-alias",
                command_manager::Host::get_by_alias(s, "probe".into()).await
            );
            denied!(
                "contains",
                command_manager::Host::contains(s, "probe".into()).await
            );
            denied!("list", command_manager::Host::list(s).await);
            denied!("list-owned", command_manager::Host::list_owned(s).await);
        }
        Capability::Scheduler => {
            denied!("delay", scheduler::Host::delay(s, 10, 1).await);
            denied!("interval", scheduler::Host::interval(s, 10, None, 1).await);
            denied!("cancel", scheduler::Host::cancel(s, 0).await);
        }
        Capability::CodecFilter => {
            denied!(
                "register-codec-filter",
                codec_registry::Host::register_codec_filter(s, codec_metadata(), 1).await
            );
            denied!(
                "unregister-codec-filter",
                codec_registry::Host::unregister_codec_filter(s, "ops".into()).await
            );
        }
        Capability::Limbo => {
            denied!(
                "register-limbo-handler",
                limbo::Host::register_limbo_handler(s, "gate".into(), 1).await
            );
        }
        Capability::BanProvider => {
            let features = ban_service::BanFeatures {
                ip_ranges: false,
                pagination: false,
            };
            denied!(
                "register-ban-provider",
                providers::Host::register_ban_provider(s, features).await
            );
        }
        Capability::PermissionProvider => {
            denied!(
                "register-permission-provider",
                providers::Host::register_permission_provider(s).await
            );
            let snapshot = permissions::PermissionSnapshot {
                rules: vec![],
                admin: false,
            };
            denied!(
                "set-snapshot",
                permissions::Host::set_snapshot(s, 1, snapshot).await
            );
            denied!("release", permissions::Host::release(s, 1).await);
        }
        other if gates_something(other) => {
            failures.push(format!("{other:?}: no gated host call to probe"));
        }
        _ => {}
    }
    failures
}

fn gates_something(capability: Capability) -> bool {
    GATES
        .iter()
        .any(|(_, _, required)| required.contains(&capability))
        || SUBSCRIBE_GATES
            .iter()
            .any(|(_, required)| *required == capability)
}

#[tokio::test]
async fn every_gated_host_call_is_refused_without_its_capability() {
    let mut failures = Vec::new();
    for &capability in Capability::ALL {
        failures.extend(unrefused_calls(capability).await);
    }
    assert!(
        failures.is_empty(),
        "host calls that were not refused:\n{}",
        failures.join("\n")
    );
}
