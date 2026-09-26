use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use infrarust_api::error::PlayerError;
use infrarust_api::event::bus::EventBusExt;
use infrarust_api::event::{BoxFuture, EventPriority};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FrameOutput,
};
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::messaging::ChannelId;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::player::{
    BossBar, BossBarControl, BossBarHandle, BossBarUpdate, ClientSettings, ConnectionResult,
    Player, ResourcePackRequest,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::services::config_service::{
    ConfigService, ConfigWriteError, ServerConfig, ServerSource,
};
use infrarust_api::services::load_balancer::{BackendStatus, LbError, LoadBalancerService};
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::test_util::{MockBanService, MockPlayer, MockPlayerRegistry};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerAddress, ServerId,
    TitleData,
};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::filter::FilterOwner;
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use tokio_util::sync::CancellationToken;

use crate::bindings::infrarust::plugin::events::EventKind;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, load_balancer,
    messaging, players, plugin_registry, proxy_info, scheduler, server_manager, text, types as wt,
};
use crate::component;
use crate::config::SandboxLimits;
use crate::deadline::Deadline;
use crate::store_state::{PluginStoreState, build_probe_state};

struct NoConfig;

impl infrarust_api::services::config_service::private::Sealed for NoConfig {}

impl ConfigService for NoConfig {
    fn get_server_config(&self, _server: &ServerId) -> Option<ServerConfig> {
        None
    }
    fn get_all_server_configs(&self) -> Vec<ServerConfig> {
        Vec::new()
    }
    fn get_server_document(&self, _server: &ServerId) -> Option<String> {
        None
    }
    fn list_server_sources(&self) -> Vec<ServerSource> {
        Vec::new()
    }
    fn get_proxy_config_document(&self) -> String {
        String::new()
    }
    fn get_effective_proxy_config_document(&self) -> String {
        String::new()
    }
    fn write_proxy_config_document(&self, _toml: &str) -> Result<(), ConfigWriteError> {
        Err(ConfigWriteError::PermissionDenied)
    }
    fn get_value(&self, key: &str) -> Option<String> {
        (key == "greeting").then(|| "hello".to_owned())
    }
}

struct NoBalancer;

impl infrarust_api::services::load_balancer::private::Sealed for NoBalancer {}

impl LoadBalancerService for NoBalancer {
    fn strategy(&self, _server: &ServerId) -> Option<String> {
        None
    }
    fn backends(&self, _server: &ServerId) -> Vec<BackendStatus> {
        Vec::new()
    }
    fn set_drained(
        &self,
        _server: &ServerId,
        _addr: &ServerAddress,
        _drained: bool,
    ) -> Result<(), LbError> {
        Ok(())
    }
    fn reset_backend(&self, _server: &ServerId, _addr: &ServerAddress) -> Result<(), LbError> {
        Ok(())
    }
}

fn context(players: Vec<Arc<dyn Player>>) -> Arc<dyn PluginContext> {
    PluginContextFactoryImpl::new(services(players), HashMap::new()).create_context("test")
}

fn services(players: Vec<Arc<dyn Player>>) -> PluginServices {
    let registry = MockPlayerRegistry::new();
    for player in players {
        registry.add_dyn(player);
    }
    PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(registry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService::new()),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(NoConfig),
        load_balancer_service: Arc::new(NoBalancer),
        plugin_registry: Arc::new(PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(CodecFilterRegistryImpl::new()),
        transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
        domain_router: Arc::new(DomainRouter::new()),
        proxy_shutdown: CancellationToken::new(),
        proxy_info: ProxyInfo::default(),
        plugins_dir: PathBuf::from("plugins"),
    }
}

fn state_with(capabilities: CapabilitySet, players: Vec<Arc<dyn Player>>) -> PluginStoreState {
    build_probe_state("test".to_owned(), &SandboxLimits::default())
        .with_capabilities(capabilities)
        .with_ctx(context(players))
}

fn text_of(message: &str) -> wt::Component {
    component::to_wit(&Component::text(message))
}

struct StalledPlayer {
    profile: GameProfile,
}

impl StalledPlayer {
    fn shared() -> Arc<dyn Player> {
        Arc::new(Self {
            profile: GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "tester".to_owned(),
                properties: vec![],
            },
        })
    }
}

impl infrarust_api::player::private::Sealed for StalledPlayer {}

impl Player for StalledPlayer {
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
        None
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn is_active(&self) -> bool {
        true
    }
    fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
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
        Box::pin(std::future::pending())
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
    let mut state = state_with(CapabilitySet::baseline(), vec![StalledPlayer::shared()]);

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
    let mut state = state_with(CapabilitySet::baseline(), vec![StalledPlayer::shared()]);
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
        vec![StalledPlayer::shared()],
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
        other => failures.push(format!("{other:?}: no gated host call to probe")),
    }
    failures
}

#[tokio::test]
async fn every_gated_host_call_is_refused_without_its_capability() {
    let mut failures = Vec::new();
    for capability in [
        Capability::Ban,
        Capability::ServerManage,
        Capability::ConfigRead,
        Capability::PlayerRead,
        Capability::PlayerWrite,
        Capability::RawPacket,
        Capability::ChatIntercept,
        Capability::EventBus,
        Capability::Command,
        Capability::Scheduler,
        Capability::CodecFilter,
        Capability::Limbo,
        Capability::ConfigWrite,
        Capability::PluginMessaging,
    ] {
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
    assert_eq!(
        load_balancer::Host::backends(&mut state, "lobby".into())
            .await
            .unwrap(),
        Ok(vec![])
    );
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
