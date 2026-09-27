#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::connection::{
    ConnectCause, PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult,
    ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{
    GameProfileRequestEvent, LoginEvent, LoginResult, PermissionsSetupEvent,
    PermissionsSetupResult, PostLoginEvent, PreLoginEvent, PreLoginResult,
};
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};
use infrarust_api::loader::PluginLoader;
use infrarust_api::plugin::Plugin;
use infrarust_api::types::{Component, ProtocolVersion, ServerId};
use infrarust_core::event_bus::{EventBusConfig, EventBusImpl};
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{
    EnvOptions, TestEnv, add_precompiled_fixture, load_enabled, loader_from_toml, nil_profile,
    read_log, write_script,
};

const SCRIPTED: &str = "scripted";
const UNAVAILABLE: &str = "A proxy plugin is unavailable. Please try again later.";
const PROMPTLY: Duration = Duration::from_secs(15);
const SHORT_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy)]
enum Access {
    PreLogin,
    Login,
    GameProfileRequest,
    PermissionsSetup,
    ServerPreConnect,
    PlayerChooseInitialServer,
    PreTransfer,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Denied,
    Untouched,
    Other(String),
}

fn player() -> Arc<dyn infrarust_api::player::Player> {
    support::session_player(
        1,
        nil_profile("Steve"),
        ProtocolVersion::MINECRAFT_1_21.raw(),
        "203.0.113.7:40000".parse().unwrap(),
    )
}

fn unavailable(reason: &Component) -> Verdict {
    if reason.to_plain() == UNAVAILABLE {
        Verdict::Denied
    } else {
        Verdict::Other(reason.to_plain())
    }
}

impl Access {
    const fn script_name(self) -> &'static str {
        match self {
            Self::PreLogin => "pre-login",
            Self::Login => "login",
            Self::GameProfileRequest => "game-profile-request",
            Self::PermissionsSetup => "permissions-setup",
            Self::ServerPreConnect => "server-pre-connect",
            Self::PlayerChooseInitialServer => "player-choose-initial-server",
            Self::PreTransfer => "pre-transfer",
        }
    }

    async fn fire(self, bus: &EventBusImpl) -> Verdict {
        match self {
            Self::PreLogin => {
                let event = bus
                    .fire(PreLoginEvent::new(
                        nil_profile("Steve"),
                        "203.0.113.7:40000".parse().unwrap(),
                        ProtocolVersion::MINECRAFT_1_21,
                        "play.example.com".to_owned(),
                    ))
                    .await;
                match event.result() {
                    PreLoginResult::Denied { reason } => unavailable(reason),
                    PreLoginResult::Allowed => Verdict::Untouched,
                    other => Verdict::Other(format!("{other:?}")),
                }
            }
            Self::Login => {
                let event = bus.fire(LoginEvent::new(player(), true)).await;
                match event.result() {
                    LoginResult::Denied { reason } => unavailable(reason),
                    LoginResult::Allowed => Verdict::Untouched,
                    other => Verdict::Other(format!("{other:?}")),
                }
            }
            Self::GameProfileRequest => {
                let event = bus
                    .fire(GameProfileRequestEvent::new(
                        nil_profile("Steve"),
                        true,
                        "203.0.113.7:40000".parse().unwrap(),
                        Some("play.example.com".to_owned()),
                        ProtocolVersion::MINECRAFT_1_21,
                    ))
                    .await;
                event.denied().map_or(Verdict::Untouched, unavailable)
            }
            Self::PermissionsSetup => {
                let event = bus.fire(PermissionsSetupEvent::new(player(), true)).await;
                match event.result() {
                    PermissionsSetupResult::Custom(checker)
                        if !checker.has_permission("scripted.anything") =>
                    {
                        Verdict::Denied
                    }
                    PermissionsSetupResult::UseDefault => Verdict::Untouched,
                    _ => Verdict::Other("a custom checker with permissions".to_owned()),
                }
            }
            Self::ServerPreConnect => {
                let event = bus
                    .fire(ServerPreConnectEvent::new(
                        player(),
                        ServerId::new("lobby"),
                        None,
                        ConnectCause::Initial,
                    ))
                    .await;
                match event.result() {
                    ServerPreConnectResult::Denied { reason } => unavailable(reason),
                    ServerPreConnectResult::Allowed => Verdict::Untouched,
                    other => Verdict::Other(format!("{other:?}")),
                }
            }
            Self::PlayerChooseInitialServer => {
                let event = bus
                    .fire(PlayerChooseInitialServerEvent::new(
                        player(),
                        ServerId::new("lobby"),
                    ))
                    .await;
                match event.result() {
                    PlayerChooseInitialServerResult::Denied { reason } => unavailable(reason),
                    PlayerChooseInitialServerResult::Allowed => Verdict::Untouched,
                    _ => Verdict::Other("redirected".to_owned()),
                }
            }
            Self::PreTransfer => {
                let event = bus
                    .fire(PreTransferEvent::new(
                        player(),
                        "old.example.com".to_owned(),
                        25565,
                        TransferOrigin::Plugin,
                    ))
                    .await;
                match event.result() {
                    PreTransferResult::Denied { reason } => unavailable(reason),
                    PreTransferResult::Allowed => Verdict::Untouched,
                    other => Verdict::Other(format!("{other:?}")),
                }
            }
        }
    }
}

struct Scripted {
    _tmp: tempfile::TempDir,
    data: std::path::PathBuf,
    env: TestEnv,
    _plugin: Box<dyn Plugin>,
}

impl Scripted {
    async fn start(script: &str, proxy_toml: &str, handler_timeout: Duration) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let plugins_dir = tmp.path().to_path_buf();
        add_precompiled_fixture(&plugins_dir, SCRIPTED).await;
        write_script(&plugins_dir, SCRIPTED, script);
        let env = support::make_env_with(
            plugins_dir.clone(),
            EnvOptions {
                bus_config: EventBusConfig {
                    handler_timeout,
                    ..EventBusConfig::default()
                },
                ..EnvOptions::default()
            }
            .grant(SCRIPTED, "chat-intercept"),
        );
        let loader = loader_from_toml(proxy_toml);
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = load_enabled(&loader, &env.factory, SCRIPTED).await;
        Self {
            data: plugins_dir.join(SCRIPTED),
            _tmp: tmp,
            env,
            _plugin: plugin,
        }
    }

    fn bus(&self) -> Arc<EventBusImpl> {
        Arc::clone(&self.env.event_bus)
    }

    fn saw(&self, event: &str) -> usize {
        read_log(&self.data)
            .iter()
            .filter(|line| line.starts_with(&format!("{event} ")))
            .count()
    }

    async fn wait_until_it_saw(&self, event: &str) {
        let deadline = Instant::now() + PROMPTLY;
        while self.saw(event) == 0 {
            assert!(Instant::now() < deadline, "the guest never saw {event}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn post_login() -> PostLoginEvent {
    PostLoginEvent::new(player())
}

async fn denied_past_the_event_deadline(access: Access) {
    let logs = LogCapture::at(Level::WARN);
    async {
        let lab = Scripted::start(
            &format!("on {} normal sleep 5000", access.script_name()),
            "[events]\nhandler_timeout = \"300ms\"\n",
            SHORT_TIMEOUT,
        )
        .await;
        let started = Instant::now();
        let verdict = access.fire(&lab.bus()).await;
        let took = started.elapsed();
        assert_eq!(verdict, Verdict::Denied, "{access:?}: past the deadline");
        assert!(
            took < SHORT_TIMEOUT,
            "{access:?}: the plugin answered before the bus gave up, after {took:?}"
        );
        let deadline = Instant::now() + PROMPTLY;
        while logs
            .matching("the call ran past the event deadline")
            .is_empty()
        {
            assert!(
                Instant::now() < deadline,
                "{access:?}: the cut is a fault with its own cause: {:?}",
                logs.lines()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    .with_subscriber(logs.clone())
    .await;
    assert_eq!(
        logs.matching("the call ran past the event deadline").len(),
        1,
        "{access:?}: {:?}",
        logs.lines()
    );
    assert_eq!(
        logs.matching("access event denied").len(),
        1,
        "{access:?}: {:?}",
        logs.lines()
    );
}

async fn denied_on_a_full_queue(access: Access) {
    let lab = Scripted::start(
        &format!(
            "on post-login normal sleep 5000\non {} normal record",
            access.script_name()
        ),
        "[events]\nhandler_timeout = \"2s\"\n\n[wasm]\nqueue_capacity = 1\n",
        Duration::from_secs(2),
    )
    .await;
    let bus = lab.bus();
    let busy = tokio::spawn({
        let bus = Arc::clone(&bus);
        async move { bus.fire(post_login()).await }
    });
    lab.wait_until_it_saw("post-login").await;
    let queued = tokio::spawn({
        let bus = Arc::clone(&bus);
        async move { bus.fire(post_login()).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    let verdict = access.fire(&bus).await;
    let took = started.elapsed();
    assert_eq!(verdict, Verdict::Denied, "{access:?}: on a full queue");
    assert!(
        took < Duration::from_millis(500),
        "{access:?}: a refused call is denied on the spot, took {took:?}"
    );
    assert_eq!(lab.saw(access.script_name()), 0, "{access:?}");
    drop(busy.await);
    drop(queued.await);
}

async fn denied_while_quarantined(access: Access) {
    let lab = Scripted::start(
        &format!(
            "on post-login normal panic\non {} normal record",
            access.script_name()
        ),
        "[wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"1h\"\nbackoff_max = \"1h\"\n",
        EventBusConfig::default().handler_timeout,
    )
    .await;
    assert_eq!(
        access.fire(&lab.bus()).await,
        Verdict::Untouched,
        "{access:?}: healthy"
    );
    lab.bus().fire(post_login()).await;
    let verdict = access.fire(&lab.bus()).await;
    assert_eq!(verdict, Verdict::Denied, "{access:?}: while quarantined");
    assert_eq!(
        lab.saw(access.script_name()),
        1,
        "{access:?}: only the healthy instance saw the event"
    );
}

async fn denied_on_a_fault(access: Access) {
    let lab = Scripted::start(
        &format!("on {} normal panic", access.script_name()),
        "",
        EventBusConfig::default().handler_timeout,
    )
    .await;
    let verdict = access.fire(&lab.bus()).await;
    assert_eq!(verdict, Verdict::Denied, "{access:?}: on a trap");
    assert_eq!(lab.saw(access.script_name()), 1, "{access:?}");
}

async fn denied_whenever_the_listener_does_not_answer(access: Access) {
    denied_past_the_event_deadline(access).await;
    denied_on_a_full_queue(access).await;
    denied_while_quarantined(access).await;
    denied_on_a_fault(access).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pre_login_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::PreLogin).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::Login).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_profile_request_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::GameProfileRequest).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permissions_setup_gets_no_permission_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::PermissionsSetup).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_pre_connect_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::ServerPreConnect).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_player_choose_initial_server_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::PlayerChooseInitialServer).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pre_transfer_is_denied_when_the_wasm_listener_does_not_answer() {
    denied_whenever_the_listener_does_not_answer(Access::PreTransfer).await;
}

fn modified_before(bus: &dyn EventBus) {
    bus.subscribe::<ChatMessageEvent, _>(EventPriority::FIRST, |event: &mut ChatMessageEvent| {
        event.set_result(ChatMessageResult::Modify {
            message: "before".to_owned(),
        });
    });
}

fn chat() -> ChatMessageEvent {
    ChatMessageEvent::new(player(), "hello".to_owned(), false, None)
}

fn kept(event: &ChatMessageEvent) -> bool {
    matches!(event.result(), ChatMessageResult::Modify { message } if message == "before")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_access_event_keeps_its_previous_result_when_the_wasm_listener_does_not_answer() {
    let slow = Scripted::start(
        "on chat-message normal sleep 5000",
        "[events]\nhandler_timeout = \"300ms\"\n",
        SHORT_TIMEOUT,
    )
    .await;
    modified_before(&*slow.bus());
    assert!(kept(&slow.bus().fire(chat()).await), "past the deadline");

    let trapping = Scripted::start(
        "on chat-message normal panic",
        "[wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"1h\"\nbackoff_max = \"1h\"\n",
        EventBusConfig::default().handler_timeout,
    )
    .await;
    modified_before(&*trapping.bus());
    assert!(kept(&trapping.bus().fire(chat()).await), "on a trap");
    assert!(
        kept(&trapping.bus().fire(chat()).await),
        "while quarantined"
    );
}
