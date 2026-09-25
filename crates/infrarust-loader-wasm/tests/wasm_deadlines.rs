#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use infrarust_api::command::CommandSource;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{PreLoginEvent, PreLoginResult};
use infrarust_api::limbo::test_util::RecordingLimboSession;
use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboHandler};
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::types::{PlayerId, ProtocolVersion, ServerId};
use infrarust_core::event_bus::EventBusConfig;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_loader_wasm::WasmPluginLoader;

use support::mock_services::{Gate, GatedBanService};
use support::{
    EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, nil_profile, read_log,
    stage,
};

const FIXTURE: &str = "deadline-probe";
const UNAVAILABLE: &str = "ban check unavailable";
const SHORT_HANDLER_TIMEOUT: Duration = Duration::from_millis(300);
const PATIENT_HANDLER_TIMEOUT: Duration = Duration::from_secs(30);
const PROMPTLY: Duration = Duration::from_secs(10);

struct Probe {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    gate: Arc<Gate>,
    _loader: WasmPluginLoader,
    _plugin: Box<dyn Plugin>,
}

async fn enable_probe(proxy_toml: &str, handler_timeout: Duration) -> Probe {
    let (tmp, plugins_dir) = stage(FIXTURE);
    let gate = Gate::new();
    let env = make_env_with(
        plugins_dir.clone(),
        EnvOptions {
            ban_service: Arc::new(GatedBanService {
                gate: Arc::clone(&gate),
            }),
            bus_config: EventBusConfig {
                handler_timeout,
                ..EventBusConfig::default()
            },
            ..EnvOptions::default()
        }
        .grant(FIXTURE, "ban")
        .grant(FIXTURE, "limbo"),
    );
    let loader = loader_from_toml(proxy_toml);
    loader.discover(&plugins_dir).await.unwrap();
    let plugin = load_enabled(&loader, &env.factory, FIXTURE).await;
    Probe {
        _tmp: tmp,
        data: plugins_dir.join(FIXTURE),
        env,
        gate,
        _loader: loader,
        _plugin: plugin,
    }
}

fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        nil_profile("Steve"),
        SocketAddr::from(([127, 0, 0, 1], 25565)),
        ProtocolVersion::MINECRAFT_1_21,
        "play.example.com".to_string(),
    )
}

fn pre_connect() -> ServerPreConnectEvent {
    ServerPreConnectEvent::new(
        support::session_player(
            1,
            nil_profile("Steve"),
            ProtocolVersion::MINECRAFT_1_21.raw(),
            "127.0.0.1:40000".parse().unwrap(),
        ),
        ServerId::new("lobby"),
        None,
        ConnectCause::Initial,
    )
}

fn login_outcome(event: &PreLoginEvent) -> String {
    match event.result() {
        PreLoginResult::Allowed => "allowed".to_string(),
        PreLoginResult::Denied { reason } => format!("denied:{}", reason.to_plain()),
        _ => "other".to_string(),
    }
}

fn connect_outcome(event: &ServerPreConnectEvent) -> String {
    match event.result() {
        ServerPreConnectResult::Allowed => "allowed".to_string(),
        ServerPreConnectResult::ConnectTo(server) => format!("connect-to:{}", server.as_str()),
        _ => "other".to_string(),
    }
}

async fn poll_once<F: Future + Unpin>(future: &mut F) -> Option<F::Output> {
    std::future::poll_fn(|cx| {
        Poll::Ready(match Pin::new(&mut *future).poll(cx) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

fn ban_gate(env: &TestEnv) -> Box<dyn LimboHandler> {
    env.factory
        .create_context(FIXTURE)
        .as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("PluginContextImpl")
        .take_limbo_handlers()
        .into_iter()
        .find(|handler| handler.name() == "ban-gate")
        .expect("the probe registers its ban-gate limbo handler")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_call_outliving_the_event_deadline_errors_in_time_for_the_guest_to_deny() {
    let probe = enable_probe(
        "[events]\nhandler_timeout = \"300ms\"\n",
        SHORT_HANDLER_TIMEOUT,
    )
    .await;

    let event = tokio::time::timeout(PROMPTLY, probe.env.event_bus.fire(pre_login()))
        .await
        .expect("the bus gives up at its handler timeout");

    assert_eq!(
        login_outcome(&event),
        format!("denied:{UNAVAILABLE}"),
        "the guest got the ban-service error before the bus deadline and failed closed"
    );
    assert_eq!(read_log(&probe.data), ["pre-login service-error"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_event_runs_within_its_own_deadline_after_a_host_call_runs_out_of_time() {
    let probe = enable_probe(
        "[events]\nhandler_timeout = \"300ms\"\n",
        SHORT_HANDLER_TIMEOUT,
    )
    .await;

    tokio::time::timeout(PROMPTLY, probe.env.event_bus.fire(pre_login()))
        .await
        .expect("the bus gives up at its handler timeout");
    let next = tokio::time::timeout(PROMPTLY, probe.env.event_bus.fire(pre_connect()))
        .await
        .expect("the bus gives up at its handler timeout");

    assert_eq!(
        connect_outcome(&next),
        "connect-to:backend-1",
        "the plugin was free again, so the next event got its answer in time"
    );
    assert_eq!(
        read_log(&probe.data),
        ["pre-login service-error", "pre-connect"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_whose_deadline_passed_while_queued_never_reaches_the_guest() {
    let probe = enable_probe(
        "[events]\nhandler_timeout = \"300ms\"\n\n[wasm]\nhost_call_timeout = \"1s\"\n",
        PATIENT_HANDLER_TIMEOUT,
    )
    .await;

    let mut check = Box::pin(dispatch_line(&probe.env.command_manager, "check"));
    assert!(poll_once(&mut check).await.is_none());
    probe.gate.entered().await;
    let mut queued = Box::pin(probe.env.event_bus.fire(pre_connect()));
    assert!(
        poll_once(&mut queued).await.is_none(),
        "the event waits in the queue behind the parked command"
    );

    assert!(tokio::time::timeout(PROMPTLY, check).await.unwrap());
    let queued = tokio::time::timeout(PROMPTLY, queued)
        .await
        .expect("an expired call is refused without running");

    assert_eq!(
        connect_outcome(&queued),
        "allowed",
        "the event outlived its 300 ms deadline in the queue, so it has no outcome"
    );
    assert_eq!(
        read_log(&probe.data),
        ["check service-error"],
        "the guest never saw the expired event"
    );
    let fresh = probe.env.event_bus.fire(pre_connect()).await;
    assert_eq!(connect_outcome(&fresh), "connect-to:backend-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_host_call_errors_before_max_call_duration_instead_of_poisoning() {
    let probe = enable_probe(
        "[plugins.deadline-probe.wasm]\nmax_call_duration = \"1s\"\n",
        PATIENT_HANDLER_TIMEOUT,
    )
    .await;

    let found = tokio::time::timeout(PROMPTLY, dispatch_line(&probe.env.command_manager, "check"))
        .await
        .expect("the command returns once its host call runs out of time");
    assert!(found);
    assert!(dispatch_line(&probe.env.command_manager, "ping").await);

    assert_eq!(
        read_log(&probe.data),
        ["check service-error", "command"],
        "the command saw the error before its deadline and the instance stayed healthy"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_limbo_host_call_errors_before_max_call_duration_so_the_handler_decides() {
    let probe = enable_probe(
        "[plugins.deadline-probe.wasm]\nmax_call_duration = \"1s\"\n",
        PATIENT_HANDLER_TIMEOUT,
    )
    .await;
    let handler = ban_gate(&probe.env);
    let session = RecordingLimboSession::new(
        PlayerId::new(1),
        nil_profile("Steve"),
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    );

    let outcome = tokio::time::timeout(PROMPTLY, handler.on_player_enter(session.as_ref()))
        .await
        .expect("the limbo call returns once its host call runs out of time");

    match outcome {
        HandlerResult::Deny(reason) => assert_eq!(
            reason.to_plain(),
            UNAVAILABLE,
            "the handler's own fail-closed decision, not the host's fallback"
        ),
        other => panic!("expected the handler to deny, got {other:?}"),
    }
}

async fn dispatch_line(commands: &CommandManagerImpl, line: &str) -> bool {
    commands.dispatch(CommandSource::Console, line).await == DispatchOutcome::Executed
}
