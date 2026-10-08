#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::error::PlayerError;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FrameOutput,
};
use infrarust_api::limbo::LimboEntryContext;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::messaging::ChannelId;
use infrarust_api::permissions::{
    Capability, CapabilitySet, PermissionDefault, PermissionNode, PermissionSubject, Tristate,
};
use infrarust_api::player::{
    BossBar, BossBarControl, BossBarHandle, BossBarUpdate, ClientSettings, ConnectionResult,
    Player, ResourcePackRequest,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::test_util::{
    MockConfigService, MockLoadBalancerService, MockPlayer, MockPlayerRegistry,
    RecordingLimboSession,
};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerAddress, ServerId,
    TitleData,
};
use infrarust_config::{PermissionsConfig, WasmQuotasConfig};
use infrarust_core::filter::FilterOwner;
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::permissions::PermissionService;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions};
use infrarust_plugin_common::capability::gates::{GATES, SUBSCRIBE_GATES};

use wasmtime::component::{Resource, ResourceTableError};

use crate::actor::InstanceRef;
use crate::bindings::infrarust::plugin::events::EventKind;
use crate::bindings::infrarust::plugin::limbo as wl;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, events, limbo,
    load_balancer, log, messaging, permission_nodes, permissions, players, plugin_registry,
    providers, proxy_info, scheduler, server_manager, text, types as wt,
};
use crate::component;
use crate::config::SandboxLimits;
use crate::consts::MAX_HOST_HANDLES;
use crate::deadline::Deadline;
use crate::store_state::{PluginStoreState, build_probe_state};

fn context(players: Vec<Arc<dyn Player>>) -> Arc<dyn PluginContext> {
    PluginContextFactoryImpl::new(services(players), HashMap::new()).create_context("test")
}

fn services(players: Vec<Arc<dyn Player>>) -> PluginServices {
    let registry = MockPlayerRegistry::new();
    for player in players {
        registry.add_dyn(player);
    }
    let lobby_backend = ServerAddress {
        host: "10.0.0.2".to_owned(),
        port: 25565,
    };
    PluginServices {
        player_registry: Arc::new(registry),
        config_service: Arc::new(MockConfigService::new().with_value("greeting", "hello")),
        load_balancer_service: Arc::new(MockLoadBalancerService::new().with_server(
            "lobby",
            "round_robin",
            [lobby_backend],
        )),
        ..PluginServices::for_tests()
    }
}

fn state_with(capabilities: CapabilitySet, players: Vec<Arc<dyn Player>>) -> PluginStoreState {
    build_probe_state("test".to_owned(), &SandboxLimits::default())
        .with_capabilities(capabilities)
        .with_ctx(context(players))
        .with_instance(InstanceRef::detached())
}

fn text_of(message: &str) -> wt::Component {
    component::to_wit(&Component::text(message))
}

fn stalled_player() -> Arc<dyn Player> {
    MockPlayer::new(1, "tester")
        .online_mode(true)
        .stalled()
        .into_arc()
}

#[tokio::test]
async fn players_are_read_by_id_name_and_uuid() {
    let steve = MockPlayer::new(7, "Steve").on_server("lobby").into_arc();
    let uuid = steve.profile().uuid;
    let mut state = state_with(CapabilitySet::baseline(), vec![steve]);

    let info = players::Host::get(&mut state, 7)
        .await
        .unwrap()
        .expect("online");
    assert_eq!(info.player.username, "Steve");
    assert_eq!(info.current_server.as_deref(), Some("lobby"));
    let by_name = players::Host::get_by_name(&mut state, "Steve".into())
        .await
        .unwrap();
    assert_eq!(by_name.map(|info| info.player.id), Some(7));
    let by_uuid = players::Host::get_by_uuid(&mut state, crate::convert::uuid_to_wit(uuid))
        .await
        .unwrap();
    assert_eq!(by_uuid.map(|info| info.player.id), Some(7));
    let unknown = players::Host::get_by_uuid(
        &mut state,
        crate::convert::uuid_to_wit(uuid::Uuid::from_u128(9)),
    )
    .await
    .unwrap();
    assert!(unknown.is_none());
    assert_eq!(players::Host::count(&mut state, None).await.unwrap(), 1);
    assert_eq!(
        players::Host::list(&mut state, Some("survival".into()))
            .await
            .unwrap()
            .len(),
        0
    );
    let listed = players::Host::list(&mut state, Some("lobby".into()))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].player.id, 7);
    assert_eq!(listed[0].player.username, "Steve");
    assert_eq!(
        listed[0].player.uuid,
        crate::convert::uuid_to_wit(uuid),
        "a listed player carries the same reference as its full record"
    );
    assert_eq!(listed[0].current_server.as_deref(), Some("lobby"));
    assert_eq!(players::Host::list(&mut state, None).await.unwrap(), listed);
}

fn connected_from(id: u16, username: &str, ip: &str) -> Arc<dyn Player> {
    MockPlayer::new(id.into(), username)
        .with_remote_addr(SocketAddr::new(ip.parse().unwrap(), 40_000 + id))
        .on_server("lobby")
        .into_arc()
}

async fn players_at(state: &mut PluginStoreState, ip: &str) -> Vec<u64> {
    let ip = crate::convert::ip_to_wit(ip.parse().unwrap());
    let found = players::Host::get_by_ip(state, ip).await.unwrap();
    let listed = players::Host::list(state, None).await.unwrap();
    assert!(
        found.iter().all(|summary| listed.contains(summary)),
        "an address lookup answers the records list answers: {found:?}"
    );
    let mut ids: Vec<u64> = found.iter().map(|summary| summary.player.id).collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn players_are_found_by_the_ip_they_connected_from() {
    let mut state = state_with(
        CapabilitySet::baseline(),
        vec![
            connected_from(7, "Steve", "203.0.113.7"),
            connected_from(8, "SteveAlt", "203.0.113.7"),
            connected_from(9, "Alex", "198.51.100.2"),
            connected_from(10, "Herobrine", "2001:db8::7"),
        ],
    );

    assert_eq!(players_at(&mut state, "203.0.113.7").await, [7, 8]);
    assert_eq!(players_at(&mut state, "::ffff:203.0.113.7").await, [7, 8]);
    assert_eq!(players_at(&mut state, "198.51.100.2").await, [9]);
    assert_eq!(players_at(&mut state, "2001:db8::7").await, [10]);
    assert!(players_at(&mut state, "192.0.2.1").await.is_empty());
    assert!(players_at(&mut state, "2001:db8::8").await.is_empty());

    let alex = players::Host::get_by_ip(&mut state, wt::IpAddress::Ipv4((198, 51, 100, 2)))
        .await
        .unwrap();
    assert_eq!(alex[0].player.username, "Alex");
    assert_eq!(alex[0].current_server.as_deref(), Some("lobby"));
}

#[tokio::test]
async fn an_ip_lookup_without_player_read_answers_like_list() {
    let mut state = state_with(
        CapabilitySet::baseline().without(Capability::PlayerRead),
        vec![connected_from(7, "Steve", "203.0.113.7")],
    );
    let found = players::Host::get_by_ip(&mut state, wt::IpAddress::Ipv4((203, 0, 113, 7)))
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(found, players::Host::list(&mut state, None).await.unwrap());
}

#[tokio::test]
async fn a_message_reaches_the_player_by_id() {
    let steve = MockPlayer::new(7, "Steve").into_arc();
    let mut state = state_with(CapabilitySet::baseline(), vec![steve.clone()]);

    let sent = players::Host::send_message(&mut state, 7, text_of("hi"))
        .await
        .unwrap();
    assert_eq!(sent, Ok(()));
    assert_eq!(steve.messages(), [Component::text("hi")]);
}

#[tokio::test]
async fn an_offline_player_is_player_gone_and_a_bad_text_is_an_argument_error() {
    let steve = MockPlayer::new(7, "Steve").into_arc();
    let mut state = state_with(CapabilitySet::baseline(), vec![steve]);

    let gone = players::Host::send_message(&mut state, 8, text_of("hi"))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.kind, wt::ErrorKind::PlayerGone);

    let broken = wt::Component { nodes: Vec::new() };
    let invalid = players::Host::send_action_bar(&mut state, 7, broken)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(invalid.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn switch_server_gives_up_long_before_the_host_call_timeout() {
    let mut state = state_with(CapabilitySet::baseline(), vec![stalled_player()]);

    let started = Instant::now();
    let result = players::Host::switch_server(&mut state, 1, "lobby".to_owned())
        .await
        .expect("a saturated session is a host-error, not a trap");
    let elapsed = started.elapsed();

    assert!(
        matches!(&result, Err(e) if e.kind == wt::ErrorKind::Timeout),
        "{result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "held the instance lock for {elapsed:?}"
    );
}

#[tokio::test]
async fn switch_server_stops_at_the_call_deadline_when_that_comes_first() {
    let mut state = state_with(CapabilitySet::baseline(), vec![stalled_player()]);
    state.begin_call(Some(Deadline::after(Duration::ZERO)));

    let result = players::Host::switch_server(&mut state, 1, "lobby".to_owned())
        .await
        .expect("a call out of time is a host-error, not a trap");

    assert!(
        matches!(&result, Err(e) if e.kind == wt::ErrorKind::Timeout && e.message.contains("deadline")),
        "{result:?}"
    );
}

#[tokio::test]
async fn player_write_calls_are_refused_without_the_capability() {
    let mut state = state_with(
        CapabilitySet::baseline().without(Capability::PlayerWrite),
        vec![stalled_player()],
    );
    let denied = |result: Result<(), wt::HostError>| matches!(&result, Err(e) if e.kind == wt::ErrorKind::PermissionDenied && e.message.contains("player-write"));

    assert!(denied(
        players::Host::switch_server(&mut state, 1, "lobby".to_owned())
            .await
            .unwrap()
    ));
    assert!(denied(
        players::Host::send_message(&mut state, 1, text_of("hi"))
            .await
            .unwrap()
    ));
    assert!(denied(
        players::Host::disconnect(&mut state, 1, text_of("bye"))
            .await
            .unwrap()
    ));
}

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

#[tokio::test]
async fn the_text_interface_uses_the_native_parser_and_serializer() {
    let mut state = build_probe_state("probe".to_owned(), &SandboxLimits::default());
    let parsed = text::Host::parse_json(&mut state, r#"{"text":"hi","bold":true}"#.into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        component::from_wit(&parsed).unwrap(),
        Component::text("hi").bold()
    );
    let bad = text::Host::parse_json(&mut state, "{".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.kind, wt::ErrorKind::InvalidArgument);
    let legacy = text::Host::parse_legacy(&mut state, "&aGreen".into())
        .await
        .unwrap();
    assert_eq!(
        text::Host::to_plain(&mut state, legacy).await.unwrap(),
        Ok("Green".to_owned())
    );
    let json = text::Host::to_json(&mut state, text_of("x")).await.unwrap();
    assert_eq!(json, Ok(Component::text("x").to_json()));
}

macro_rules! expect {
    ($failures:ident, $capability:expr, $call:literal, $result:expr, $pattern:pat $(if $guard:expr)?) => {{
        let result = $result;
        if !matches!(result, $pattern $(if $guard)?) {
            $failures.push(format!("{:?} {}: {:?}", $capability, $call, result));
        }
    }};
}

fn nobody() -> ban_service::BanTarget {
    ban_service::BanTarget::Username("nobody".to_owned())
}

fn is_denied(result: &Result<impl std::fmt::Debug, wt::HostError>, capability: Capability) -> bool {
    matches!(result, Err(e) if e.kind == wt::ErrorKind::PermissionDenied
        && e.message == format!("missing capability: {}", capability.to_kebab()))
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

fn address() -> wt::ServerAddress {
    wt::ServerAddress {
        host: "10.0.0.2".into(),
        port: 25565,
    }
}

fn channel() -> wt::ChannelId {
    wt::ChannelId {
        modern: Some("test:echo".into()),
        legacy: None,
    }
}

fn packet_filter() -> event_bus::PacketFilter {
    event_bus::PacketFilter {
        packet_id: 3,
        state: wt::ConnectionState::Play,
        direction: wt::PacketDirection::Serverbound,
    }
}

fn boss_bar() -> players::BossBar {
    players::BossBar {
        title: text_of("boss"),
        progress: 0.25,
        color: players::BossBarColor::Red,
        overlay: players::BossBarOverlay::Notched10,
        flags: players::BossBarFlags::DARKEN_SCREEN,
    }
}

fn resource_pack() -> players::ResourcePackRequest {
    players::ResourcePackRequest {
        id: wt::Uuid { hi: 0, lo: 7 },
        url: "https://example.com/pack.zip".into(),
        hash: None,
        required: true,
        prompt: Some(text_of("please")),
    }
}

fn command_spec() -> command_manager::CommandSpec {
    command_manager::CommandSpec {
        name: "probe".to_owned(),
        aliases: vec![],
        description: String::new(),
        usage: None,
        permission: None,
        hidden: false,
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

#[derive(Default)]
struct Recorded {
    messages: Mutex<Vec<(String, Vec<u8>, bool)>>,
    bars: Mutex<Vec<(uuid::Uuid, Option<BossBarUpdate>)>>,
    cookies: Mutex<HashMap<String, Bytes>>,
    packs: Mutex<Vec<ResourcePackRequest>>,
}

impl BossBarControl for Recorded {
    fn update(&self, id: uuid::Uuid, update: BossBarUpdate) -> Result<(), PlayerError> {
        self.bars.lock().unwrap().push((id, Some(update)));
        Ok(())
    }

    fn hide(&self, id: uuid::Uuid) -> Result<(), PlayerError> {
        self.bars.lock().unwrap().push((id, None));
        Ok(())
    }
}

struct RecordingPlayer {
    profile: GameProfile,
    seen: Arc<Recorded>,
}

impl RecordingPlayer {
    fn shared() -> (Arc<dyn Player>, Arc<Recorded>) {
        let seen = Arc::new(Recorded::default());
        let player = Arc::new(Self {
            profile: GameProfile {
                uuid: uuid::Uuid::from_u128(1),
                username: "Steve".to_owned(),
                properties: vec![],
            },
            seen: Arc::clone(&seen),
        });
        (player, seen)
    }
}

impl infrarust_api::player::private::Sealed for RecordingPlayer {}

impl Player for RecordingPlayer {
    fn id(&self) -> PlayerId {
        PlayerId::new(1)
    }
    fn profile(&self) -> &GameProfile {
        &self.profile
    }
    fn protocol_version(&self) -> ProtocolVersion {
        ProtocolVersion::MINECRAFT_1_21
    }
    fn remote_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }
    fn current_server(&self) -> Option<ServerId> {
        Some(ServerId::new("lobby"))
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn is_active(&self) -> bool {
        true
    }
    fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn send_message(&self, _message: Component) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_title(&self, _title: TitleData) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_action_bar(&self, _message: Component) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_packet(&self, _packet: RawPacket) -> Result<(), PlayerError> {
        Ok(())
    }
    fn switch_server(&self, _target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        Box::pin(async { Ok(()) })
    }
    fn is_online_mode(&self) -> bool {
        true
    }
    fn has_permission(&self, _permission: &str) -> bool {
        false
    }
    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn connected_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
    fn settings(&self) -> Option<ClientSettings> {
        Some(ClientSettings::new("fr_fr"))
    }
    fn known_channels(&self) -> Vec<String> {
        vec!["test:echo".to_owned()]
    }
    fn send_plugin_message(&self, channel: &ChannelId, data: Bytes) -> Result<(), PlayerError> {
        let entry = (channel.to_string(), data.to_vec(), false);
        self.seen.messages.lock().unwrap().push(entry);
        Ok(())
    }
    fn send_plugin_message_to_backend(
        &self,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<(), PlayerError> {
        let entry = (channel.to_string(), data.to_vec(), true);
        self.seen.messages.lock().unwrap().push(entry);
        Ok(())
    }
    fn connect(&self, target: ServerId) -> BoxFuture<'_, Result<ConnectionResult, PlayerError>> {
        Box::pin(async move {
            Ok(if target.as_str() == "lobby" {
                ConnectionResult::AlreadyConnected
            } else {
                ConnectionResult::Denied(Component::text("full"))
            })
        })
    }
    fn show_boss_bar(&self, _bar: BossBar) -> Result<BossBarHandle, PlayerError> {
        let control: Arc<dyn BossBarControl> = Arc::clone(&self.seen) as Arc<dyn BossBarControl>;
        Ok(BossBarHandle::new(uuid::Uuid::from_u128(42), control))
    }
    fn send_resource_pack(&self, pack: ResourcePackRequest) -> Result<(), PlayerError> {
        self.seen.packs.lock().unwrap().push(pack);
        Ok(())
    }
    fn store_cookie(&self, key: &str, data: Bytes) -> Result<(), PlayerError> {
        self.seen
            .cookies
            .lock()
            .unwrap()
            .insert(key.to_owned(), data);
        Ok(())
    }
    fn request_cookie(&self, key: &str) -> BoxFuture<'_, Result<Option<Bytes>, PlayerError>> {
        let cookie = self.seen.cookies.lock().unwrap().get(key).cloned();
        Box::pin(async move { Ok(cookie) })
    }
}

fn messenger() -> CapabilitySet {
    CapabilitySet::baseline().with(Capability::PluginMessaging)
}

#[tokio::test]
async fn plugin_messages_reach_the_client_and_the_backend_on_registered_channels() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(messenger(), vec![player]);

    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(())
    );
    let channels = messaging::Host::channels(&mut state)
        .await
        .unwrap()
        .unwrap();
    assert!(channels.contains(&channel()), "{channels:?}");

    messaging::Host::send_to_player(&mut state, 1, channel(), b"hi".to_vec())
        .await
        .unwrap()
        .unwrap();
    messaging::Host::send_to_backend(&mut state, 1, channel(), b"up".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *seen.messages.lock().unwrap(),
        [
            ("test:echo".to_owned(), b"hi".to_vec(), false),
            ("test:echo".to_owned(), b"up".to_vec(), true),
        ]
    );

    let invalid = wt::ChannelId {
        modern: Some("Bad Channel".into()),
        legacy: None,
    };
    let refused = messaging::Host::send_to_player(&mut state, 1, invalid, vec![])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);

    let nobody = messaging::Host::send_to_server(&mut state, "lobby".into(), channel(), vec![])
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(nobody.kind, wt::ErrorKind::Unavailable, "{nobody:?}");

    assert_eq!(
        messaging::Host::unregister_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(true)
    );
}

#[tokio::test]
async fn player_info_carries_settings_and_known_channels() {
    let (player, _) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);
    let info = players::Host::get(&mut state, 1).await.unwrap().unwrap();
    assert_eq!(info.known_channels, ["test:echo"]);
    assert_eq!(info.settings.map(|s| s.locale).as_deref(), Some("fr_fr"));
}

#[tokio::test]
async fn a_boss_bar_is_shown_updated_and_hidden_by_its_id() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);

    let bar = players::Host::show_boss_bar(&mut state, 1, boss_bar())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::convert::uuid_from_wit(bar),
        uuid::Uuid::from_u128(42)
    );
    players::Host::update_boss_bar(&mut state, bar, players::BossBarUpdate::Progress(0.75))
        .await
        .unwrap()
        .unwrap();
    players::Host::hide_boss_bar(&mut state, bar)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *seen.bars.lock().unwrap(),
        [
            (
                uuid::Uuid::from_u128(42),
                Some(BossBarUpdate::Progress(0.75))
            ),
            (uuid::Uuid::from_u128(42), None),
        ]
    );
    let gone = players::Host::hide_boss_bar(&mut state, bar)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.kind, wt::ErrorKind::NotFound);
}

#[tokio::test]
async fn cookies_round_trip_and_connect_reports_the_switch_outcome() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);

    players::Host::store_cookie(&mut state, 1, "infrarust:token".into(), b"t".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        players::Host::request_cookie(&mut state, 1, "infrarust:token".into())
            .await
            .unwrap(),
        Ok(Some(b"t".to_vec()))
    );
    assert_eq!(
        players::Host::connect(&mut state, 1, "lobby".into())
            .await
            .unwrap(),
        Ok(players::ConnectionResult::AlreadyConnected)
    );
    assert_eq!(
        players::Host::connect(&mut state, 1, "full".into())
            .await
            .unwrap(),
        Ok(players::ConnectionResult::Denied(text_of("full")))
    );

    players::Host::send_resource_pack(&mut state, 1, resource_pack())
        .await
        .unwrap()
        .unwrap();
    let pack = seen.packs.lock().unwrap()[0].clone();
    assert_eq!(pack.id, uuid::Uuid::from_u128(7));
    assert_eq!(pack.prompt, Some(Component::text("please")));

    let mut bad = resource_pack();
    bad.hash = Some("not-a-sha1".into());
    let refused = players::Host::send_resource_pack(&mut state, 1, bad)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);

    let unsupported = players::Host::transfer(&mut state, 1, address())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(unsupported.kind, wt::ErrorKind::Unsupported);
}

#[tokio::test]
async fn fire_named_returns_what_the_native_listeners_decided() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    let ctx = state.services().unwrap();
    let seen_source = Arc::new(Mutex::new(String::new()));
    let source = Arc::clone(&seen_source);
    ctx.event_bus()
        .subscribe::<NamedEvent, _>(EventPriority::NORMAL, move |event| {
            if event.name == "echo" {
                *source.lock().unwrap() = event.source_plugin.clone();
                event.respond("text/plain", event.payload.clone());
                event.cancel();
            }
        });

    let answered = event_bus::Host::fire_named(
        &mut state,
        "echo".into(),
        "text/plain".into(),
        b"hi".to_vec(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(answered.cancelled);
    let response = answered.response.expect("a response");
    assert_eq!(response.payload, b"hi");
    assert_eq!(*seen_source.lock().unwrap(), "test");

    let ignored = event_bus::Host::fire_named(&mut state, "other".into(), "x".into(), vec![])
        .await
        .unwrap()
        .unwrap();
    assert!(!ignored.cancelled);
    assert_eq!(ignored.response, None);
}

#[tokio::test]
async fn subscribe_packets_needs_a_filter_and_registers_one_native_listener_per_filter() {
    let mut state = state_with(
        CapabilitySet::baseline().with(Capability::RawPacket),
        vec![],
    );
    let empty = event_bus::Host::subscribe_packets(&mut state, vec![], 128)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(empty.kind, wt::ErrorKind::InvalidArgument);
    let listener = event_bus::Host::subscribe_packets(&mut state, vec![packet_filter()], 128)
        .await
        .unwrap()
        .unwrap();
    let ctx = state.services().unwrap();
    assert!(ctx.event_bus().has_packet_listeners(
        3,
        infrarust_api::event::ConnectionState::Play,
        infrarust_api::events::packet::PacketDirection::Serverbound
    ));
    assert_eq!(
        event_bus::Host::unsubscribe(&mut state, listener)
            .await
            .unwrap(),
        Ok(true)
    );
    assert!(!ctx.event_bus().has_packet_listeners(
        3,
        infrarust_api::event::ConnectionState::Play,
        infrarust_api::events::packet::PacketDirection::Serverbound
    ));
    let refused = event_bus::Host::subscribe(&mut state, EventKind::RawPacket, 128)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn proxy_info_and_the_plugin_registry_answer_without_any_capability() {
    let granted = CapabilitySet::default()
        .with(Capability::Ban)
        .with(Capability::PluginMessaging);
    let mut state = state_with(granted, vec![]);
    let mut capabilities = proxy_info::Host::granted_capabilities(&mut state)
        .await
        .unwrap();
    capabilities.sort_by_key(|c| *c as u8);
    assert_eq!(
        capabilities,
        [wt::Capability::Ban, wt::Capability::PluginMessaging]
    );
    let details = proxy_info::Host::details(&mut state).await.unwrap();
    assert_eq!(details.bind.port, 25565);
    assert_eq!(
        details.unknown_domain_behavior,
        proxy_info::UnknownDomainBehavior::DefaultMotd
    );
    assert!(
        plugin_registry::Host::list(&mut state)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        plugin_registry::Host::get(&mut state, "nobody".into())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn load_balancer_reads_and_config_writes_reach_the_native_services() {
    let mut state = state_with(CapabilitySet::native_trusted(), vec![]);
    let backends = load_balancer::Host::backends(&mut state, "lobby".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backends.len(), 1);
    assert_eq!(backends[0].address, address());
    assert_eq!(
        load_balancer::Host::set_drained(&mut state, "lobby".into(), address(), true)
            .await
            .unwrap(),
        Ok(())
    );
    let refused = config_service::Host::write_proxy_config_document(&mut state, "bind = 1".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::PermissionDenied);
    assert_eq!(
        config_service::Host::list_server_sources(&mut state)
            .await
            .unwrap(),
        Ok(vec![])
    );
}

fn routed_state(capabilities: CapabilitySet) -> PluginStoreState {
    let router = Arc::new(infrarust_core::routing::DomainRouter::new());
    for (file, document) in [
        (
            "lobby.toml",
            "id = \"lobby\"\naddresses = [\"10.0.0.1:25565\"]\ndomains = [\"*.example.com\"]\n",
        ),
        (
            "survival.toml",
            "id = \"survival\"\naddresses = [\"10.0.0.2:25565\"]\ndomains = [\"survival.example.com\"]\n",
        ),
    ] {
        router.add(
            infrarust_core::provider::ProviderId::new("file", file),
            toml::from_str(document).unwrap(),
        );
    }
    let config_service = infrarust_core::services::config_service::ConfigServiceImpl::new(
        router,
        std::path::PathBuf::from("infrarust.toml"),
        Arc::new(toml::from_str::<infrarust_config::ProxyConfig>("").unwrap()),
    );
    let services = PluginServices {
        config_service: Arc::new(config_service),
        ..services(vec![])
    };
    build_probe_state("test".to_owned(), &SandboxLimits::default())
        .with_capabilities(capabilities)
        .with_ctx(PluginContextFactoryImpl::new(services, HashMap::new()).create_context("test"))
        .with_instance(InstanceRef::detached())
}

async fn server_for(state: &mut PluginStoreState, domain: &str) -> Option<String> {
    config_service::Host::get_server_by_domain(state, domain.into())
        .await
        .unwrap()
        .unwrap()
        .map(|config| config.id)
}

#[tokio::test]
async fn a_domain_finds_the_server_the_proxy_routes_it_to() {
    let mut state = routed_state(CapabilitySet::baseline());

    for (domain, server) in [
        ("survival.example.com", Some("survival")),
        ("Survival.Example.COM.", Some("survival")),
        ("hub.example.com", Some("lobby")),
        ("HUB.example.com.", Some("lobby")),
        ("example.com", None),
        ("survival.example.org", None),
    ] {
        assert_eq!(
            server_for(&mut state, domain).await.as_deref(),
            server,
            "{domain}"
        );
    }

    let survival =
        config_service::Host::get_server_by_domain(&mut state, "survival.example.com".into())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        survival,
        config_service::Host::get_server(&mut state, "survival".into())
            .await
            .unwrap()
            .unwrap(),
        "a domain answers the record get-server answers for its server"
    );
    assert_eq!(
        survival.map(|config| config.domains),
        Some(vec!["survival.example.com".to_owned()])
    );
}

#[tokio::test]
async fn a_domain_lookup_without_config_read_is_refused_like_get_server() {
    let mut state = routed_state(CapabilitySet::baseline().without(Capability::ConfigRead));
    let by_domain =
        config_service::Host::get_server_by_domain(&mut state, "survival.example.com".into())
            .await
            .unwrap();
    assert!(
        is_denied(&by_domain, Capability::ConfigRead),
        "{by_domain:?}"
    );
    assert_eq!(
        by_domain,
        config_service::Host::get_server(&mut state, "survival".into())
            .await
            .unwrap()
    );
}

struct PassFactory(&'static str);

struct Pass;

impl CodecFilterFactory for PassFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new(self.0)
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(Pass)
    }
}

impl CodecFilterInstance for Pass {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

fn codec_state(factory: &PluginContextFactoryImpl, plugin_id: &str) -> PluginStoreState {
    build_probe_state(plugin_id.to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::baseline().with(Capability::CodecFilter))
        .with_ctx(factory.create_context(plugin_id))
}

#[tokio::test]
async fn a_codec_filter_can_only_be_unregistered_by_the_plugin_that_owns_it() {
    let registry = Arc::new(CodecFilterRegistryImpl::new());
    let grant = |plugin_id: &str| {
        let permissions = PluginPermissions {
            permissions: vec![Capability::CodecFilter.to_kebab().to_owned()],
            ..PluginPermissions::default()
        };
        (plugin_id.to_owned(), permissions)
    };
    let factory = PluginContextFactoryImpl::new(
        PluginServices {
            codec_filter_registry: Arc::clone(&registry),
            ..services(vec![])
        },
        HashMap::from([grant("owner"), grant("intruder")]),
    );
    let mut owner = codec_state(&factory, "owner");
    let mut intruder = codec_state(&factory, "intruder");
    owner
        .ctx()
        .and_then(|ctx| ctx.codec_filters())
        .expect("the owner holds the codec-filter capability")
        .register(Box::new(PassFactory("shared")))
        .unwrap();

    let refused = codec_registry::Host::unregister_codec_filter(&mut intruder, "shared".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::Conflict);
    assert!(refused.message.contains("owner"), "{}", refused.message);
    assert_eq!(
        registry.owner_of("shared"),
        Some(FilterOwner::plugin("owner"))
    );
    let missing = codec_registry::Host::unregister_codec_filter(&mut intruder, "missing".into())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.kind, wt::ErrorKind::NotFound);

    assert_eq!(
        codec_registry::Host::unregister_codec_filter(&mut owner, "shared".into())
            .await
            .unwrap(),
        Ok(())
    );
    assert!(registry.is_empty());
}

fn quota_state(quotas: WasmQuotasConfig, grants: &[Capability]) -> PluginStoreState {
    let permissions = PluginPermissions {
        permissions: grants.iter().map(|c| c.to_kebab().to_owned()).collect(),
        ..PluginPermissions::default()
    };
    let factory = PluginContextFactoryImpl::new(
        services(vec![]),
        HashMap::from([("test".to_owned(), permissions)]),
    );
    let capabilities = grants
        .iter()
        .fold(CapabilitySet::baseline(), |set, capability| {
            set.with(*capability)
        });
    let sandbox = SandboxLimits {
        quotas,
        ..SandboxLimits::default()
    };
    build_probe_state("test".to_owned(), &sandbox)
        .with_capabilities(capabilities)
        .with_ctx(factory.create_context("test"))
        .with_instance(InstanceRef::detached())
}

fn quotas(adjust: impl FnOnce(&mut WasmQuotasConfig)) -> WasmQuotasConfig {
    let mut quotas = WasmQuotasConfig::default();
    adjust(&mut quotas);
    quotas
}

fn is_limit_exceeded<T: std::fmt::Debug>(result: &Result<T, wt::HostError>, key: &str) -> bool {
    matches!(result, Err(e) if e.kind == wt::ErrorKind::LimitExceeded
        && e.message.ends_with(&format!("(quotas.{key})")))
}

#[tokio::test]
async fn the_listener_quota_counts_every_native_listener_and_unsubscribing_frees_room() {
    let mut state = quota_state(quotas(|q| q.event_listeners = 3), &[Capability::RawPacket]);
    let first = event_bus::Host::subscribe(&mut state, EventKind::PostLogin, 0)
        .await
        .unwrap()
        .unwrap();
    let two_filters = vec![packet_filter(), packet_filter()];
    event_bus::Host::subscribe_packets(&mut state, two_filters.clone(), 0)
        .await
        .unwrap()
        .unwrap();
    let over = event_bus::Host::subscribe_packets(&mut state, two_filters, 0)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "event_listeners"), "{over:?}");
    let named = event_bus::Host::subscribe_named(&mut state, "full".into(), 0)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&named, "event_listeners"), "{named:?}");

    assert_eq!(
        event_bus::Host::unsubscribe(&mut state, first)
            .await
            .unwrap(),
        Ok(true)
    );
    assert!(
        event_bus::Host::subscribe_named(&mut state, "room".into(), 0)
            .await
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn the_task_quota_counts_live_tasks_and_a_fired_delay_frees_its_room() {
    let mut state = quota_state(quotas(|q| q.scheduled_tasks = 2), &[]);
    let interval = scheduler::Host::interval(&mut state, 60_000, None, 1)
        .await
        .unwrap()
        .unwrap();
    scheduler::Host::delay(&mut state, 60_000, 2)
        .await
        .unwrap()
        .unwrap();
    let over = scheduler::Host::delay(&mut state, 60_000, 3).await.unwrap();
    assert!(is_limit_exceeded(&over, "scheduled_tasks"), "{over:?}");

    assert_eq!(
        scheduler::Host::cancel(&mut state, interval).await.unwrap(),
        Ok(())
    );
    scheduler::Host::delay(&mut state, 0, 4)
        .await
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let again = scheduler::Host::delay(&mut state, 60_000, 5).await.unwrap();
        if again.is_ok() {
            break;
        }
        assert!(is_limit_exceeded(&again, "scheduled_tasks"), "{again:?}");
        assert!(
            Instant::now() < deadline,
            "the fired delay still took room in the quota"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_command_quota_lets_a_plugin_replace_a_command_and_unregistering_frees_room() {
    let mut state = quota_state(quotas(|q| q.commands = 2), &[]);
    let named = |name: &str| command_manager::CommandSpec {
        name: name.to_owned(),
        ..command_spec()
    };
    for name in ["alpha", "beta"] {
        command_manager::Host::register(&mut state, named(name), 1)
            .await
            .unwrap()
            .unwrap();
    }
    let over = command_manager::Host::register(&mut state, named("gamma"), 2)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "commands"), "{over:?}");
    assert!(
        command_manager::Host::register(&mut state, named("Beta"), 3)
            .await
            .unwrap()
            .is_ok(),
        "registering a held name again replaces it"
    );

    assert_eq!(
        command_manager::Host::unregister(&mut state, "alpha".into())
            .await
            .unwrap(),
        Ok(())
    );
    assert!(
        command_manager::Host::register(&mut state, named("gamma"), 4)
            .await
            .unwrap()
            .is_ok()
    );
}

struct Builtin;

impl CommandHandler for Builtin {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

fn command_state(factory: &PluginContextFactoryImpl, plugin_id: &str) -> PluginStoreState {
    build_probe_state(plugin_id.to_owned(), &SandboxLimits::default())
        .with_capabilities(CapabilitySet::baseline())
        .with_ctx(factory.create_context(plugin_id))
        .with_instance(InstanceRef::detached())
}

fn spec_with_aliases(name: &str, aliases: &[&str]) -> command_manager::CommandSpec {
    command_manager::CommandSpec {
        name: name.to_owned(),
        aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
        ..command_spec()
    }
}

fn label_of(info: &command_manager::CommandInfo) -> String {
    info.plugin_id.as_ref().map_or_else(
        || info.spec.name.clone(),
        |plugin| format!("{plugin}:{}", info.spec.name),
    )
}

fn labels_of(infos: &[command_manager::CommandInfo]) -> Vec<String> {
    infos.iter().map(label_of).collect()
}

#[tokio::test]
async fn command_lookups_see_every_command_and_list_owned_answers_only_the_callers() {
    let shared = services(vec![]);
    shared
        .command_manager
        .register_builtin(CommandSpec::new("infrarust").alias("ir"), Box::new(Builtin));
    let factory = PluginContextFactoryImpl::new(shared, HashMap::new());
    let mut guest = command_state(&factory, "guest");
    let mut other = command_state(&factory, "other");
    let warp = command_manager::CommandSpec {
        description: "Teleport to a warp".to_owned(),
        usage: Some("/warp <name>".to_owned()),
        permission: Some("warps.use".to_owned()),
        hidden: true,
        ..spec_with_aliases("Warp", &["W", "ir"])
    };
    command_manager::Host::register(&mut other, warp, 1)
        .await
        .unwrap()
        .unwrap();
    command_manager::Host::register(&mut guest, spec_with_aliases("home", &["h"]), 2)
        .await
        .unwrap()
        .unwrap();

    macro_rules! found {
        ($call:ident, $state:expr, $label:literal) => {
            command_manager::Host::$call(&mut $state, $label.into())
                .await
                .unwrap()
                .unwrap()
                .as_ref()
                .map(label_of)
        };
    }
    assert_eq!(
        command_manager::Host::get(&mut guest, "w".into())
            .await
            .unwrap(),
        Ok(Some(command_manager::CommandInfo {
            spec: command_manager::CommandSpec {
                name: "warp".to_owned(),
                aliases: vec!["w".to_owned()],
                description: "Teleport to a warp".to_owned(),
                usage: Some("/warp <name>".to_owned()),
                permission: Some("warps.use".to_owned()),
                hidden: true,
            },
            plugin_id: Some("other".to_owned()),
        })),
        "another plugin's command arrives with its whole spec and the aliases it was granted"
    );
    assert_eq!(found!(get, guest, "WARP").as_deref(), Some("other:warp"));
    assert_eq!(
        found!(get, guest, "Other:Warp").as_deref(),
        Some("other:warp")
    );
    assert_eq!(found!(get, guest, "ir").as_deref(), Some("infrarust"));
    assert_eq!(found!(get, guest, "nope"), None);

    assert_eq!(
        found!(get_by_name, guest, "warp").as_deref(),
        Some("other:warp")
    );
    assert_eq!(
        found!(get_by_name, guest, "guest:home").as_deref(),
        Some("guest:home")
    );
    assert_eq!(
        found!(get_by_name, guest, "infrarust").as_deref(),
        Some("infrarust")
    );
    assert_eq!(found!(get_by_name, guest, "w"), None);
    assert_eq!(found!(get_by_name, guest, "ir"), None);

    assert_eq!(
        found!(get_by_alias, guest, "W").as_deref(),
        Some("other:warp")
    );
    assert_eq!(
        found!(get_by_alias, guest, "ir").as_deref(),
        Some("infrarust")
    );
    assert_eq!(
        found!(get_by_alias, other, "h").as_deref(),
        Some("guest:home")
    );
    assert_eq!(found!(get_by_alias, guest, "warp"), None);
    assert_eq!(found!(get_by_alias, guest, "other:warp"), None);

    for (label, answer) in [
        ("h", true),
        ("other:warp", true),
        ("IR", true),
        ("nope", false),
    ] {
        assert_eq!(
            command_manager::Host::contains(&mut guest, label.into())
                .await
                .unwrap(),
            Ok(answer),
            "{label}"
        );
    }

    let listed = command_manager::Host::list(&mut guest)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        labels_of(&listed),
        ["guest:home", "infrarust", "other:warp"]
    );
    let owned = |infos: Result<Vec<command_manager::CommandInfo>, wt::HostError>| {
        labels_of(&infos.unwrap())
    };
    assert_eq!(
        owned(command_manager::Host::list_owned(&mut guest).await.unwrap()),
        ["guest:home"]
    );
    assert_eq!(
        owned(command_manager::Host::list_owned(&mut other).await.unwrap()),
        ["other:warp"]
    );

    command_manager::Host::unregister(&mut guest, "home".into())
        .await
        .unwrap()
        .unwrap();
    assert!(owned(command_manager::Host::list_owned(&mut guest).await.unwrap()).is_empty());
    assert_eq!(found!(get, other, "h"), None);
}

#[tokio::test]
async fn the_channel_quota_counts_distinct_channels_and_unregistering_frees_room() {
    let mut state = quota_state(
        quotas(|q| q.plugin_channels = 1),
        &[Capability::PluginMessaging],
    );
    let other = wt::ChannelId {
        modern: Some("test:other".into()),
        legacy: None,
    };
    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        messaging::Host::register_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(()),
        "registering a held channel again takes no room"
    );
    let over = messaging::Host::register_channel(&mut state, other.clone())
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "plugin_channels"), "{over:?}");

    assert_eq!(
        messaging::Host::unregister_channel(&mut state, channel())
            .await
            .unwrap(),
        Ok(true)
    );
    assert_eq!(
        messaging::Host::register_channel(&mut state, other)
            .await
            .unwrap(),
        Ok(())
    );
}

#[tokio::test]
async fn the_limbo_quota_refuses_a_new_handler_name_but_not_one_already_held() {
    let mut state = quota_state(quotas(|q| q.limbo_handlers = 1), &[Capability::Limbo]);
    assert_eq!(
        wl::Host::register_limbo_handler(&mut state, "gate".into(), 1)
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        wl::Host::register_limbo_handler(&mut state, "gate".into(), 2)
            .await
            .unwrap(),
        Ok(())
    );
    let over = wl::Host::register_limbo_handler(&mut state, "other".into(), 3)
        .await
        .unwrap();
    assert!(is_limit_exceeded(&over, "limbo_handlers"), "{over:?}");
}

struct NativeGate;

impl infrarust_api::limbo::LimboHandler for NativeGate {
    fn name(&self) -> &str {
        "taken"
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn infrarust_api::limbo::LimboSession,
    ) -> BoxFuture<'a, infrarust_api::limbo::HandlerResult> {
        Box::pin(async { infrarust_api::limbo::HandlerResult::Accept })
    }
}

#[tokio::test]
async fn a_refused_limbo_handler_takes_no_room_and_is_refused_again() {
    let mut owner = quota_state(quotas(|q| q.limbo_handlers = 1), &[Capability::Limbo]);
    let shared_ctx = owner.services().unwrap();
    shared_ctx
        .register_limbo_handler(Box::new(NativeGate))
        .unwrap();
    let first = wl::Host::register_limbo_handler(&mut owner, "taken".into(), 1)
        .await
        .unwrap();
    assert!(
        matches!(&first, Err(e) if e.kind == wt::ErrorKind::Conflict),
        "{first:?}"
    );
    let again = wl::Host::register_limbo_handler(&mut owner, "taken".into(), 2)
        .await
        .unwrap();
    assert!(
        matches!(&again, Err(e) if e.kind == wt::ErrorKind::Conflict),
        "{again:?}"
    );
    assert_eq!(
        wl::Host::register_limbo_handler(&mut owner, "free".into(), 3)
            .await
            .unwrap(),
        Ok(())
    );
}

struct NodeHost {
    permissions: Arc<PermissionService>,
    factory: PluginContextFactoryImpl,
}

impl NodeHost {
    fn new() -> Self {
        let permissions = Arc::new(PermissionService::new_sync(&PermissionsConfig::default()));
        let factory = PluginContextFactoryImpl::new(services(vec![]), HashMap::new())
            .with_permissions(Arc::clone(&permissions));
        Self {
            permissions,
            factory,
        }
    }

    fn wasm(&self, plugin_id: &str, quotas: WasmQuotasConfig) -> PluginStoreState {
        let sandbox = SandboxLimits {
            quotas,
            ..SandboxLimits::default()
        };
        build_probe_state(plugin_id.to_owned(), &sandbox)
            .with_capabilities(CapabilitySet::default())
            .with_ctx(self.factory.create_context(plugin_id))
            .with_instance(InstanceRef::detached())
    }

    fn native(&self, plugin_id: &str, name: &str, default: PermissionDefault) {
        self.factory
            .context(plugin_id)
            .register_permission_node(PermissionNode::new(name, default))
            .unwrap();
    }

    async fn player_value(&self, node: &str) -> Tristate {
        let steve = PermissionSubject::player(
            PlayerId::new(7),
            GameProfile {
                uuid: uuid::Uuid::from_u128(7),
                username: "Steve".to_owned(),
                properties: vec![],
            },
            true,
            SocketAddr::from(([203, 0, 113, 7], 51234)),
        );
        let checker = self.permissions.create_checker(&steve).await;
        self.permissions.value(checker.as_ref(), node)
    }
}

fn wit_node(
    name: &str,
    default: permission_nodes::PermissionDefault,
) -> permission_nodes::PermissionNode {
    permission_nodes::PermissionNode {
        name: name.to_owned(),
        description: String::new(),
        default,
    }
}

async fn register_node(
    state: &mut PluginStoreState,
    node: permission_nodes::PermissionNode,
) -> Result<(), wt::HostError> {
    permission_nodes::Host::register(state, node).await.unwrap()
}

#[tokio::test]
async fn a_wasm_plugin_registers_its_own_node_and_a_player_gets_its_default() {
    let host = NodeHost::new();
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    let fly = permission_nodes::PermissionNode {
        description: "Fly in the lobby".to_owned(),
        ..wit_node(" Test.Fly ", permission_nodes::PermissionDefault::True)
    };
    assert_eq!(register_node(&mut state, fly).await, Ok(()));
    let staff = wit_node("test.staff", permission_nodes::PermissionDefault::Admin);
    assert_eq!(register_node(&mut state, staff).await, Ok(()));

    assert_eq!(host.player_value("test.fly").await, Tristate::True);
    assert_eq!(host.player_value("test.staff").await, Tristate::False);
    let info = host
        .factory
        .context("reader")
        .permission_node("test.fly")
        .unwrap();
    assert_eq!(info.plugin_id.as_deref(), Some("test"));
    assert_eq!(info.node.name, "test.fly");
    assert_eq!(info.node.description, "Fly in the lobby");
    assert_eq!(info.node.default, PermissionDefault::True);
}

#[tokio::test]
async fn a_wasm_plugin_may_only_register_nodes_in_its_own_namespace() {
    let host = NodeHost::new();
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    for name in ["otherplugin.x", "fly", "infrarust.x", "testing.x", "test"] {
        let refused = register_node(
            &mut state,
            wit_node(name, permission_nodes::PermissionDefault::True),
        )
        .await
        .unwrap_err();
        assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument, "{name}");
        assert!(
            refused.message.contains("start with 'test.'"),
            "{name}: {}",
            refused.message
        );
    }
    assert_eq!(
        host.player_value("otherplugin.x").await,
        Tristate::Undefined
    );
    assert_eq!(host.player_value("fly").await, Tristate::Undefined);
    assert_eq!(
        host.factory.context("reader").permission_nodes().len(),
        1,
        "only the proxy's own infrarust.admin is registered"
    );
    assert_eq!(
        register_node(
            &mut state,
            wit_node("TEST.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "the name is normalized before its namespace is checked"
    );
}

#[tokio::test]
async fn a_wasm_plugin_still_meets_the_native_rules_inside_its_namespace() {
    let host = NodeHost::new();
    host.native("other", "test.shared", PermissionDefault::False);
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    let taken = register_node(
        &mut state,
        wit_node("test.shared", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(taken.kind, wt::ErrorKind::Conflict);
    assert!(taken.message.contains("'other'"), "{}", taken.message);
    assert_eq!(host.player_value("test.shared").await, Tristate::False);
    let invalid = register_node(
        &mut state,
        wit_node("test.*", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.kind, wt::ErrorKind::InvalidArgument);

    let mut proxy_named = host.wasm("infrarust", WasmQuotasConfig::default());
    for name in ["infrarust.fly", "infrarust.admin"] {
        let reserved = register_node(
            &mut proxy_named,
            wit_node(name, permission_nodes::PermissionDefault::True),
        )
        .await
        .unwrap_err();
        assert_eq!(reserved.kind, wt::ErrorKind::Conflict, "{name}");
    }
    assert_eq!(
        host.player_value("infrarust.fly").await,
        Tristate::Undefined
    );
}

#[tokio::test]
async fn registering_an_owned_node_again_updates_it_and_takes_no_more_room() {
    let host = NodeHost::new();
    let one = || quotas(|q| q.permission_nodes = 1);
    let mut state = host.wasm("test", one());
    assert_eq!(
        register_node(
            &mut state,
            wit_node("test.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(())
    );
    let moved = permission_nodes::PermissionNode {
        description: "Fly anywhere".to_owned(),
        ..wit_node("TEST.FLY", permission_nodes::PermissionDefault::Admin)
    };
    assert_eq!(register_node(&mut state, moved).await, Ok(()));
    let info = permission_nodes::Host::get(&mut state, "test.fly".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        info.node.default,
        permission_nodes::PermissionDefault::Admin
    );
    assert_eq!(info.node.description, "Fly anywhere");
    assert_eq!(host.player_value("test.fly").await, Tristate::False);

    let mut recovered = host.wasm("test", one());
    assert_eq!(
        register_node(
            &mut recovered,
            wit_node("test.fly", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "a fresh instance's on_enable registers the node it still owns"
    );
    assert_eq!(host.player_value("test.fly").await, Tristate::True);
    let full = register_node(
        &mut recovered,
        wit_node("test.walk", permission_nodes::PermissionDefault::True),
    )
    .await;
    assert!(is_limit_exceeded(&full, "permission_nodes"), "{full:?}");
}

#[tokio::test]
async fn the_node_quota_counts_only_the_nodes_the_plugin_holds() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    host.native("other", "other.ban", PermissionDefault::Admin);
    let mut state = host.wasm("test", quotas(|q| q.permission_nodes = 2));
    for name in ["test.a", "test.b"] {
        assert_eq!(
            register_node(
                &mut state,
                wit_node(name, permission_nodes::PermissionDefault::True)
            )
            .await,
            Ok(()),
            "{name}"
        );
    }
    let over = register_node(
        &mut state,
        wit_node("test.c", permission_nodes::PermissionDefault::True),
    )
    .await;
    assert!(is_limit_exceeded(&over, "permission_nodes"), "{over:?}");
    assert_eq!(host.player_value("test.c").await, Tristate::Undefined);
    let foreign = register_node(
        &mut state,
        wit_node("other.c", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(foreign.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn get_and_list_see_the_proxys_and_every_plugins_nodes_without_any_capability() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    let mut state = host.wasm("test", WasmQuotasConfig::default());
    register_node(
        &mut state,
        wit_node("test.fly", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap();

    let kick = permission_nodes::Host::get(&mut state, " Other.Kick ".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kick.plugin_id.as_deref(), Some("other"));
    assert_eq!(
        kick.node.default,
        permission_nodes::PermissionDefault::Admin
    );
    let admin = permission_nodes::Host::get(&mut state, "infrarust.admin".into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admin.plugin_id, None);
    assert_eq!(
        admin.node.default,
        permission_nodes::PermissionDefault::False
    );
    assert_eq!(
        permission_nodes::Host::get(&mut state, "nobody.x".into())
            .await
            .unwrap(),
        None
    );
    let listed: Vec<(String, Option<String>)> = permission_nodes::Host::list(&mut state)
        .await
        .unwrap()
        .into_iter()
        .map(|info| (info.node.name, info.plugin_id))
        .collect();
    assert_eq!(
        listed,
        [
            ("infrarust.admin".to_owned(), None),
            ("other.kick".to_owned(), Some("other".to_owned())),
            ("test.fly".to_owned(), Some("test".to_owned())),
        ]
    );

    let mut inspected = build_probe_state("test".to_owned(), &SandboxLimits::default());
    let unavailable = register_node(
        &mut inspected,
        wit_node("test.walk", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap_err();
    assert_eq!(unavailable.kind, wt::ErrorKind::Unavailable);
    assert!(
        permission_nodes::Host::list(&mut inspected)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_disabled_wasm_plugin_takes_its_nodes_along() {
    let host = NodeHost::new();
    host.native("other", "other.kick", PermissionDefault::Admin);
    let mut state = host.wasm("test", quotas(|q| q.permission_nodes = 1));
    register_node(
        &mut state,
        wit_node("test.fly", permission_nodes::PermissionDefault::True),
    )
    .await
    .unwrap();
    let mut reader = host.wasm("reader", WasmQuotasConfig::default());

    host.factory.context("test").cleanup();

    assert_eq!(
        permission_nodes::Host::get(&mut reader, "test.fly".into())
            .await
            .unwrap(),
        None
    );
    assert_eq!(host.player_value("test.fly").await, Tristate::Undefined);
    let names: Vec<String> = permission_nodes::Host::list(&mut reader)
        .await
        .unwrap()
        .into_iter()
        .map(|info| info.node.name)
        .collect();
    assert_eq!(names, ["infrarust.admin", "other.kick"]);
    assert_eq!(
        register_node(
            &mut state,
            wit_node("test.walk", permission_nodes::PermissionDefault::True)
        )
        .await,
        Ok(()),
        "the removed node no longer counts against the quota"
    );
}

struct DebugSink;

impl tracing::Subscriber for DebugSink {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::DEBUG
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::DEBUG)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[tokio::test]
async fn the_guest_is_told_the_most_verbose_level_the_proxy_logs() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    let sink = tracing::Dispatch::new(DebugSink);
    let told = log::Host::max_level(&mut state).await.unwrap();
    let expected = super::enabled_level().map(|level| match level {
        tracing::Level::ERROR => log::Level::Error,
        tracing::Level::WARN => log::Level::Warn,
        tracing::Level::INFO => log::Level::Info,
        tracing::Level::DEBUG => log::Level::Debug,
        _ => log::Level::Trace,
    });
    assert_eq!(told, expected);
    assert!(
        matches!(told, Some(log::Level::Debug | log::Level::Trace)),
        "a live subscriber at debug makes debug lines worth sending, got {told:?}"
    );
    drop(sink);
}

#[tokio::test]
async fn the_ping_fields_are_read_only_while_a_ping_is_handled() {
    let mut state = state_with(CapabilitySet::baseline(), vec![]);
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        None
    );
    assert_eq!(events::Host::ping_favicon(&mut state).await.unwrap(), None);
    assert!(
        events::Host::ping_player_sample(&mut state)
            .await
            .unwrap()
            .is_empty()
    );

    state.set_event_details(Some(Arc::new(crate::events::EventDetails::Ping(
        crate::events::PingDetails {
            description: Component::text("motd"),
            favicon: Some("data:image/png;base64,AAAA".to_owned()),
            player_sample: vec![("Notch".to_owned(), uuid::Uuid::from_u128(7))],
        },
    ))));
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        Some(text_of("motd"))
    );
    assert_eq!(
        events::Host::ping_favicon(&mut state)
            .await
            .unwrap()
            .as_deref(),
        Some("data:image/png;base64,AAAA")
    );
    let sample = events::Host::ping_player_sample(&mut state).await.unwrap();
    assert_eq!(sample.len(), 1);
    assert_eq!(sample[0].name, "Notch");

    state.begin_call(None);
    assert_eq!(
        events::Host::ping_description(&mut state).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn acquiring_handles_in_a_loop_is_refused_at_the_host_handle_cap() {
    let mut state = state_with(CapabilitySet::default(), vec![]);
    let session = RecordingLimboSession::new(
        PlayerId::new(1),
        GameProfile {
            uuid: uuid::Uuid::nil(),
            username: "tester".to_owned(),
            properties: vec![],
        },
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    );
    let session = state.push_limbo_session(session).unwrap();

    let mut held = Vec::new();
    let refused = loop {
        let borrowed = Resource::new_borrow(session.rep());
        match wl::HostLimboSession::acquire_handle(&mut state, borrowed).await {
            Ok(handle) => held.push(handle),
            Err(error) => break error,
        }
        assert!(
            held.len() < MAX_HOST_HANDLES,
            "the handle cap was never reached"
        );
    };
    assert_eq!(held.len(), MAX_HOST_HANDLES - 1);
    assert!(
        matches!(
            refused.downcast_ref::<ResourceTableError>(),
            Some(ResourceTableError::Full)
        ),
        "{refused:?}"
    );

    let released = held.pop().unwrap();
    wl::HostLimboSessionHandle::drop(&mut state, released)
        .await
        .unwrap();
    let borrowed = Resource::new_borrow(session.rep());
    assert!(
        wl::HostLimboSession::acquire_handle(&mut state, borrowed)
            .await
            .is_ok()
    );
}
