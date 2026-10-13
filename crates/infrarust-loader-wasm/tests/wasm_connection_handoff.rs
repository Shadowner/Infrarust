#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::claim::{
    LoginClaimEvent, LoginClaimResult, RawConnectionEvent, RawConnectionResult,
};
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, HandshakeIntent,
};
use infrarust_api::events::messaging::{
    LoginPluginRequestEvent, LoginPluginRequestResult, LoginPluginResponseEvent,
    LoginPluginResponseResult,
};
use infrarust_api::handoff::{Handoff, HandoffTarget};
use infrarust_api::player::Player;
use infrarust_api::test_util::{MockConfigService, MockPlayer};
use infrarust_api::types::{ProtocolVersion, ServerAddress, ServerId};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::plugin::manager::PluginManager;
use infrarust_core::services::command_manager::DispatchOutcome;

use support::{EnvOptions, console, loader_from_toml, make_env_with, stage};

const PROBE: &str = "claim-probe";

const NETWORK: &str =
    "[plugins.claim-probe.wasm.network]\nallow = [\"files.example:443\", \"10.0.0.5:30037\"]\n";

const FILES_ONLY: &str = "[plugins.claim-probe.wasm.network]\nallow = [\"files.example:443\"]\n";

fn modded_at(port: u16) -> MockConfigService {
    MockConfigService::new().with_server(MockConfigService::server_at(
        "modded",
        ServerAddress {
            host: "10.0.0.5".to_owned(),
            port,
        },
    ))
}

fn log_lines(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(PROBE).join("claims.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn enable_block(phase: &str, granted: &str) -> Vec<String> {
    let mut lines = vec![format!("enable {phase}")];
    if phase == "initial" {
        lines.push(format!("register [88, 70, 69, 82] {granted}"));
        lines.push(format!("register [88, 70, 79, 75] {granted}"));
        lines.push(format!(
            "register [16, 0] {}",
            if granted == "ok" {
                "invalid-argument"
            } else {
                granted
            }
        ));
    } else {
        lines.push(format!("register [88, 70, 79, 75] {granted}"));
    }
    lines.push(format!(
        "unregister-unknown {}",
        if granted == "ok" {
            "not-found"
        } else {
            granted
        }
    ));
    lines.push(format!("subscribe raw-connection {granted}"));
    lines.push(format!("subscribe login-claim {granted}"));
    lines.push("subscribe login-plugin-request ok".to_owned());
    lines.push("subscribe login-plugin-response ok".to_owned());
    lines.push("subscribe connection-handshake ok".to_owned());
    lines
}

fn steve() -> Arc<dyn Player> {
    MockPlayer::new(7, "Steve").into_arc()
}

async fn raw(bus: &EventBusImpl, command: u8) -> RawConnectionResult {
    let mut data = b"XFER".to_vec();
    data.push(command);
    let event = RawConnectionEvent::new(
        1,
        "203.0.113.7:50000".parse().unwrap(),
        "0.0.0.0:25565".parse().unwrap(),
        &b"XFER"[..],
        data,
    );
    bus.fire(event).await.result().clone()
}

async fn claim(bus: &EventBusImpl, username: &str, layout_matches: bool) -> LoginClaimResult {
    let event = LoginClaimEvent::new(
        "203.0.113.7:50000".parse().unwrap(),
        ProtocolVersion::new(767),
        username,
        Bytes::from_static(b"\x05Steve"),
    )
    .with_host("modded.test", Some("modded.test".into()), 25565)
    .with_server(Some(ServerId::new("modded")))
    .with_layout_matches(layout_matches);
    bus.fire(event).await.result().clone()
}

async fn request(bus: &EventBusImpl, channel: &str, reachable: bool) -> LoginPluginRequestResult {
    let event = LoginPluginRequestEvent::new(
        steve(),
        ServerId::new("modded"),
        5,
        channel,
        Bytes::from_static(b"abc"),
        reachable,
    );
    bus.fire(event).await.result().clone()
}

async fn response(bus: &EventBusImpl, channel: &str) -> LoginPluginResponseResult {
    let event = LoginPluginResponseEvent::new(
        steve(),
        ServerId::new("modded"),
        5,
        channel,
        true,
        Bytes::from_static(b"xy"),
    );
    bus.fire(event).await.result().clone()
}

async fn handshake(bus: &EventBusImpl, host: &str) -> ConnectionHandshakeResult {
    let event = ConnectionHandshakeEvent::new(
        "203.0.113.7:50000".parse().unwrap(),
        HandshakeIntent::Login,
        ProtocolVersion::new(767),
    )
    .with_host(host, Some(host.to_owned()), 25565);
    bus.fire(event).await.result().clone()
}

async fn wait_for_recovery(dir: &Path) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !log_lines(dir).contains(&"enable recovered".to_owned()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the probe never recovered: {:?}", log_lines(dir)));
}

fn prefixes(claims: &infrarust_core::handoff::ConnectionClaims) -> Vec<Bytes> {
    claims.prefixes()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_granted_guest_claims_prefixes_and_answers_every_new_hook() {
    let (_tmp, dir) = stage(PROBE);
    let env = make_env_with(
        dir.clone(),
        EnvOptions {
            config_service: Arc::new(modded_at(25565)),
            ..EnvOptions::default()
        }
        .grant(PROBE, "connection-handoff")
        .grant(PROBE, "plugin-messaging")
        .grant(PROBE, "network"),
    );
    let bus = Arc::clone(&env.event_bus);
    let claims = Arc::clone(&env.connection_claims);
    let commands = Arc::clone(&env.command_manager);
    let factory = Arc::new(env.factory);
    let mut manager = PluginManager::new(vec![Box::new(loader_from_toml(NETWORK))]);
    manager.discover_all(&dir).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::clone(&factory)).await;
    assert!(errors.is_empty(), "{errors:?}");

    assert_eq!(log_lines(&dir), enable_block("initial", "ok"));
    assert_eq!(
        prefixes(&claims),
        [Bytes::from_static(b"XFER"), Bytes::from_static(b"XFOK")]
    );

    assert_eq!(raw(&bus, b'P').await, RawConnectionResult::Pass);
    assert_eq!(raw(&bus, b'N').await, RawConnectionResult::NeedMore(32));
    assert_eq!(raw(&bus, b'R').await, RawConnectionResult::Reject);
    assert_eq!(
        raw(&bus, b'H').await,
        RawConnectionResult::Handoff(
            Handoff::to(HandoffTarget::server_port("modded", 30037))
                .with_prefix(&b"PRE"[..])
                .with_reply(&b"OK"[..])
                .without_replay()
                .with_proxy_protocol(true)
        )
    );
    assert_eq!(
        raw(&bus, b'A').await,
        RawConnectionResult::Handoff(Handoff::to(HandoffTarget::address("files.example", 443)))
    );
    assert_eq!(
        raw(&bus, b'B').await,
        RawConnectionResult::Pass,
        "an address outside the network allow-list is ignored"
    );
    assert_eq!(raw(&bus, b'?').await, RawConnectionResult::Pass);

    assert_eq!(claim(&bus, "pass", true).await, LoginClaimResult::Pass);
    assert_eq!(
        claim(&bus, "route", true).await,
        LoginClaimResult::Handoff(Handoff::route())
    );
    assert_eq!(
        claim(&bus, "server", true).await,
        LoginClaimResult::Handoff(
            Handoff::to(HandoffTarget::server("modded"))
                .with_reply(&b"\x05Steve"[..])
                .with_proxy_protocol(false)
        )
    );
    assert_eq!(
        claim(&bus, "mismatch", false).await,
        LoginClaimResult::Handoff(Handoff::to(HandoffTarget::server("lenient")))
    );
    assert_eq!(claim(&bus, "mismatch", true).await, LoginClaimResult::Pass);

    assert_eq!(
        request(&bus, "probe:forward", true).await,
        LoginPluginRequestResult::Forward
    );
    assert_eq!(
        request(&bus, "probe:replace", true).await,
        LoginPluginRequestResult::Replace(Bytes::from_static(b"cba"))
    );
    assert_eq!(
        request(&bus, "probe:respond", true).await,
        LoginPluginRequestResult::Respond(Some(Bytes::from_static(b"answer")))
    );
    assert_eq!(
        request(&bus, "probe:respond", false).await,
        LoginPluginRequestResult::Respond(Some(Bytes::from_static(b"unreachable")))
    );
    assert_eq!(
        request(&bus, "probe:none", true).await,
        LoginPluginRequestResult::Respond(None)
    );
    assert_eq!(
        request(&bus, "fml:loginwrapper", true).await,
        LoginPluginRequestResult::Forward
    );

    assert_eq!(
        response(&bus, "probe:replace").await,
        LoginPluginResponseResult::Replace {
            successful: false,
            data: Bytes::from_static(b"\x00\x00\x00\x05xy"),
        }
    );
    assert_eq!(
        response(&bus, "probe:other").await,
        LoginPluginResponseResult::Forward
    );

    assert_eq!(
        handshake(&bus, "handoff.test").await,
        ConnectionHandshakeResult::Handoff(
            Handoff::to(HandoffTarget::server("modded")).with_prefix(&b"HS"[..])
        )
    );
    assert_eq!(
        handshake(&bus, "lobby.test").await,
        ConnectionHandshakeResult::Allow
    );

    assert_eq!(
        commands.dispatch(console(), "claimtrap").await,
        DispatchOutcome::Executed
    );
    wait_for_recovery(&dir).await;
    let mut both = enable_block("initial", "ok");
    both.extend(enable_block("recovered", "ok"));
    assert_eq!(log_lines(&dir), both);
    assert_eq!(
        prefixes(&claims),
        [Bytes::from_static(b"XFOK")],
        "a recovered instance starts from no claim and keeps only what it registers again"
    );
    assert_eq!(raw(&bus, b'R').await, RawConnectionResult::Reject);

    manager.disable_plugin(PROBE).await.unwrap();
    assert!(claims.is_empty(), "a disabled plugin keeps no claim");
    assert_eq!(raw(&bus, b'R').await, RawConnectionResult::Pass);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_without_connection_handoff_cannot_claim_or_hand_off() {
    let (_tmp, dir) = stage(PROBE);
    let env = make_env_with(
        dir.clone(),
        EnvOptions::default().grant(PROBE, "plugin-messaging"),
    );
    let bus = Arc::clone(&env.event_bus);
    let claims = Arc::clone(&env.connection_claims);
    let factory = Arc::new(env.factory);
    let mut manager = PluginManager::new(vec![Box::new(loader_from_toml(""))]);
    manager.discover_all(&dir).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::clone(&factory)).await;
    assert!(errors.is_empty(), "{errors:?}");

    assert_eq!(
        log_lines(&dir),
        enable_block("initial", "permission-denied")
    );
    assert!(claims.is_empty());
    assert_eq!(raw(&bus, b'R').await, RawConnectionResult::Pass);
    assert_eq!(claim(&bus, "route", true).await, LoginClaimResult::Pass);
    assert_eq!(
        handshake(&bus, "handoff.test").await,
        ConnectionHandshakeResult::Allow,
        "a handoff outcome without connection-handoff is ignored"
    );
    assert_eq!(
        request(&bus, "probe:respond", true).await,
        LoginPluginRequestResult::Respond(Some(Bytes::from_static(b"answer")))
    );

    manager.disable_plugin(PROBE).await.unwrap();
}

async fn server_port_handoff(servers: MockConfigService) -> RawConnectionResult {
    let (_tmp, dir) = stage(PROBE);
    let env = make_env_with(
        dir.clone(),
        EnvOptions {
            config_service: Arc::new(servers),
            ..EnvOptions::default()
        }
        .grant(PROBE, "connection-handoff")
        .grant(PROBE, "network"),
    );
    let bus = Arc::clone(&env.event_bus);
    let factory = Arc::new(env.factory);
    let mut manager = PluginManager::new(vec![Box::new(loader_from_toml(FILES_ONLY))]);
    manager.discover_all(&dir).await.unwrap();
    let errors = manager.load_and_enable_all(Arc::clone(&factory)).await;
    assert!(errors.is_empty(), "{errors:?}");
    let result = raw(&bus, b'H').await;
    manager.disable_plugin(PROBE).await.unwrap();
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_port_handoff_off_the_minecraft_port_needs_the_allow_list() {
    assert_eq!(
        server_port_handoff(modded_at(25565)).await,
        RawConnectionResult::Pass,
        "port 30037 of the server's host is not in the allow-list"
    );
    assert!(
        matches!(
            server_port_handoff(modded_at(30037)).await,
            RawConnectionResult::Handoff(_)
        ),
        "the server's own port is reached like a server target"
    );
    assert_eq!(
        server_port_handoff(MockConfigService::new()).await,
        RawConnectionResult::Pass,
        "a server the proxy does not know is refused"
    );
}
