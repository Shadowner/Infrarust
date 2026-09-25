use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::player::Player;
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
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use tokio_util::sync::CancellationToken;

use crate::bindings::infrarust::plugin::events::EventKind;
use crate::bindings::infrarust::plugin::{
    ban_service, codec_registry, command_manager, config_service, event_bus, limbo, players,
    scheduler, server_manager, text, types as wt,
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
    let registry = MockPlayerRegistry::new();
    for player in players {
        registry.add_dyn(player);
    }
    let services = PluginServices {
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
    };
    PluginContextFactoryImpl::new(services, HashMap::new()).create_context("test")
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
        }
        Capability::ChatIntercept => {
            denied!(
                "subscribe(chat-message)",
                event_bus::Host::subscribe(s, EventKind::ChatMessage, 128).await
            );
        }
        Capability::EventBus => {
            denied!(
                "subscribe",
                event_bus::Host::subscribe(s, EventKind::PostLogin, 128).await
            );
            denied!("unsubscribe", event_bus::Host::unsubscribe(s, 1).await);
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
    ] {
        failures.extend(unrefused_calls(capability).await);
    }
    assert!(
        failures.is_empty(),
        "host calls that were not refused:\n{}",
        failures.join("\n")
    );
}
