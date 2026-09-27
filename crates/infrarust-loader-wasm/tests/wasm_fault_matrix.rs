#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fault_lab;
mod support;

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::ChatMessageResult;
use infrarust_api::events::named::NamedEvent;
use infrarust_api::limbo::HandlerResult;
use infrarust_api::loader::PluginLoader;
use infrarust_api::permissions::{PermissionSubject, Tristate};
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, RawPacket};
use infrarust_config::{PermissionProviderSelection, PermissionsConfig};
use infrarust_core::ban::BanManager;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use infrarust_core::permissions::PermissionService;
use infrarust_core::registry::ConnectionRegistry;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use fault_lab::faults::{self, Mode};
use fault_lab::{LAB, Lab, LabOptions, LabPlugin, PROMPTLY};
use support::log_capture::LogCapture;

const LIMITS: &str = "[wasm]\ncpu_budget = \"200ms\"\nmax_call_duration = \"1s\"\nmemory_limit_mb = 16\ncodec_cpu_budget = \"100ms\"\n\n[wasm.recovery]\nmax_restarts = 1000\n";
const FAILED: &str = "wasm plugin instance failed";
const RECOVERED: &str = "wasm plugin recovered";

type Answer = Pin<Box<dyn Future<Output = String> + Send>>;
type Trigger = dyn Fn(Arc<Lab>) -> Answer + Sync;

struct Site {
    key: &'static str,
    op: &'static str,
    directives: &'static str,
    grants: &'static [&'static str],
    fallback: &'static str,
    healthy: &'static str,
}

fn cause_matches(mode: Mode, error: &str) -> bool {
    match mode {
        Mode::Sleep => error.contains("max_call_duration"),
        Mode::Spin => error.contains("interrupt"),
        _ => error.contains("trapped"),
    }
}

async fn drive(site: &Site, trigger: &Trigger, options: LabOptions) {
    let logs = LogCapture::at(Level::INFO);
    async {
        let mut plugin = LabPlugin::lab(site.directives);
        for grant in site.grants {
            plugin = plugin.grant(grant);
        }
        let lab = Arc::new(Lab::start(vec![plugin], options).await);
        assert_eq!(
            trigger(Arc::clone(&lab)).await,
            site.healthy,
            "{}: the healthy call answers",
            site.key
        );
        for (round, mode) in Mode::TRAPS.into_iter().enumerate() {
            let errors = logs.matching(FAILED).len();
            let recovered = logs.matching(RECOVERED).len();
            lab.mark(LAB);
            lab.set_faults(
                LAB,
                &format!("{}\n{} {}", site.directives, site.key, mode.as_str()),
            );
            let started = Instant::now();
            let answer = tokio::time::timeout(PROMPTLY, trigger(Arc::clone(&lab)))
                .await
                .unwrap_or_else(|_| panic!("{} {mode:?}: the faulting call hangs", site.key));
            let took = started.elapsed();
            assert_eq!(
                answer, site.fallback,
                "{} {mode:?}: the faulting call gets the documented fallback",
                site.key
            );
            lab.wait_for(&format!("{} {mode:?} recovery", site.key), || {
                logs.matching(RECOVERED).len() > recovered
            })
            .await;
            let new_errors: Vec<String> = logs.matching(FAILED)[errors..].to_vec();
            assert_eq!(
                new_errors.len(),
                1,
                "{} {mode:?}: one error per fault: {new_errors:?} guest log {:?}",
                site.key,
                lab.log(LAB)
            );
            assert!(
                new_errors[0].contains(&format!("op=\"{}\"", site.op))
                    && cause_matches(mode, &new_errors[0]),
                "{} {mode:?}: the error names the call and the cause: {new_errors:?}",
                site.key
            );
            let generation = format!("generation={}", round + 2);
            assert!(
                logs.matching(RECOVERED)
                    .last()
                    .is_some_and(|line| line.contains(&generation)),
                "{} {mode:?}: {generation} expected: {:?}",
                site.key,
                logs.matching(RECOVERED)
            );
            lab.set_faults(LAB, site.directives);
            lab.mark(LAB);
            assert_eq!(
                trigger(Arc::clone(&lab)).await,
                site.healthy,
                "{} {mode:?}: the fresh instance answers the next call (fault took {took:?})",
                site.key
            );
        }
        let panics = logs.matching("panicked");
        assert!(panics.is_empty(), "no host panic: {panics:?}");
    }
    .with_subscriber(logs.clone())
    .await;
}

fn boxed(future: impl Future<Output = String> + Send + 'static) -> Answer {
    Box::pin(future)
}

fn chat_trigger(lab: Arc<Lab>) -> Answer {
    boxed(async move {
        let event = lab.event_bus.fire(fault_lab::chat()).await;
        match event.result() {
            ChatMessageResult::Modify { message } => format!("modify {message}"),
            ChatMessageResult::Allow => "allowed".to_owned(),
            other => format!("{other:?}"),
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_an_event_handler_leaves_the_event_unchanged_and_recovers() {
    let site = Site {
        key: "event:chat-message",
        op: "handle-event",
        directives: "",
        grants: &[],
        fallback: "allowed",
        healthy: "modify fault-lab",
    };
    drive(&site, &chat_trigger, limits()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_other_event_families_leaves_their_result_unchanged() {
    let pre_login = Site {
        key: "event:pre-login",
        op: "handle-event",
        directives: "",
        grants: &[],
        fallback: "allowed",
        healthy: "denied fault-lab",
    };
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            let event = lab.event_bus.fire(fault_lab::pre_login()).await;
            match event.result() {
                infrarust_api::events::lifecycle::PreLoginResult::Allowed => "allowed".to_owned(),
                infrarust_api::events::lifecycle::PreLoginResult::Denied { reason } => {
                    format!("denied {}", reason.to_plain())
                }
                _ => "other".to_owned(),
            }
        })
    };
    drive(&pre_login, &trigger, limits()).await;

    let pre_connect = Site {
        key: "event:server-pre-connect",
        op: "handle-event",
        directives: "",
        grants: &[],
        fallback: "allowed",
        healthy: "redirect fault-lab",
    };
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            let event = lab
                .event_bus
                .fire(infrarust_api::events::connection::ServerPreConnectEvent::new(
                    fault_lab::player(1),
                    infrarust_api::types::ServerId::from("lobby"),
                    None,
                    infrarust_api::events::connection::ConnectCause::Initial,
                ))
                .await;
            match event.result() {
                infrarust_api::events::connection::ServerPreConnectResult::Allowed => {
                    "allowed".to_owned()
                }
                infrarust_api::events::connection::ServerPreConnectResult::Redirect(server) => {
                    format!("redirect {server}")
                }
                _ => "other".to_owned(),
            }
        })
    };
    drive(&pre_connect, &trigger, limits()).await;
}

fn limits() -> LabOptions {
    LabOptions {
        proxy_toml: LIMITS.to_owned(),
        ..LabOptions::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_command_does_nothing_and_recovers() {
    let site = Site {
        key: "command",
        op: "handle-command",
        directives: "",
        grants: &[],
        fallback: "absent",
        healthy: "present",
    };
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            lab.clear_log(LAB);
            lab.dispatch("lab probe").await;
            if lab.log(LAB).iter().any(|line| line == "command probe") {
                "present".to_owned()
            } else {
                "absent".to_owned()
            }
        })
    };
    drive(&site, &trigger, limits()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_tab_completion_gives_no_suggestion_and_recovers() {
    let site = Site {
        key: "complete",
        op: "tab-complete",
        directives: "",
        grants: &[],
        fallback: "",
        healthy: "lab-suggestion",
    };
    let trigger = |lab: Arc<Lab>| boxed(async move { lab.complete("lab ").await.join(",") });
    drive(&site, &trigger, limits()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_named_event_listener_leaves_no_response_and_recovers() {
    let site = Site {
        key: "named:lab-ping",
        op: "handle-event",
        directives: "listen lab-ping",
        grants: &[],
        fallback: "-",
        healthy: "fault-lab",
    };
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            let event = lab
                .event_bus
                .fire(NamedEvent::new("lab-ping", "text/plain", b"x".to_vec()))
                .await;
            event.response.as_ref().map_or_else(
                || "-".to_owned(),
                |response| String::from_utf8_lossy(&response.payload).into_owned(),
            )
        })
    };
    drive(&site, &trigger, limits()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_scheduled_task_is_recovered_and_the_next_instance_runs_its_task() {
    let site = Site {
        key: "task",
        op: "on-scheduled-task",
        directives: "interval 400",
        grants: &[],
        fallback: "ran",
        healthy: "ran",
    };
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            lab.wait_for("the scheduled task", || {
                lab.since_mark(LAB).iter().any(|line| line == "task")
            })
            .await;
            "ran".to_owned()
        })
    };
    drive(&site, &trigger, limits()).await;
}

fn limbo_site(key: &'static str, op: &'static str, fallback: &'static str, healthy: &'static str) -> Site {
    Site {
        key,
        op,
        directives: "limbo",
        grants: &["limbo"],
        fallback,
        healthy,
    }
}

fn handler_result(result: &HandlerResult) -> String {
    match result {
        HandlerResult::Deny(reason) => format!("deny {}", reason.to_plain()),
        HandlerResult::Hold => "hold".to_owned(),
        other => format!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_limbo_entry_denies_the_player_and_recovers() {
    let site = limbo_site(
        "limbo-enter",
        "limbo-on-player-enter",
        "deny Limbo handler unavailable",
        "hold",
    );
    let trigger = |lab: Arc<Lab>| {
        boxed(async move {
            let handler = lab.limbo_handler(LAB, faults::LIMBO_HANDLER);
            let session = fault_lab::limbo_session(7);
            handler_result(&handler.on_player_enter(session.as_ref()).await)
        })
    };
    drive(&site, &trigger, limits()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_the_other_limbo_callbacks_does_nothing_and_recovers() {
    for (key, op) in [
        ("limbo-command", "limbo-on-command"),
        ("limbo-chat", "limbo-on-chat"),
        ("limbo-disconnect", "limbo-on-disconnect"),
        ("limbo-session-end", "limbo-on-session-end"),
    ] {
        let site = limbo_site(key, op, "absent", "present");
        let trigger = move |lab: Arc<Lab>| {
            boxed(async move {
                lab.clear_log(LAB);
                let handler = lab.limbo_handler(LAB, faults::LIMBO_HANDLER);
                let session = fault_lab::limbo_session(7);
                match key {
                    "limbo-command" => handler.on_command(session.as_ref(), "noop", &[]).await,
                    "limbo-chat" => handler.on_chat(session.as_ref(), "hello").await,
                    "limbo-disconnect" => handler.on_disconnect(PlayerId::new(7)).await,
                    _ => {
                        handler
                            .on_session_end(
                                PlayerId::new(7),
                                infrarust_api::limbo::SessionEndReason::Released,
                            )
                            .await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
                let ran = lab.log(LAB).iter().any(|line| line == key);
                let recovered = lab
                    .log(LAB)
                    .iter()
                    .any(|line| line.starts_with("enable recovered"));
                if ran && !recovered { "present" } else { "absent" }.to_owned()
            })
        };
        drive(&site, &trigger, limits()).await;
    }
}

fn subject(id: u64) -> PermissionSubject {
    PermissionSubject::player(
        PlayerId::new(id),
        GameProfile {
            uuid: uuid::Uuid::from_u128(u128::from(id)),
            username: format!("player{id}"),
            properties: vec![],
        },
        true,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_ban_check_refuses_the_login_and_recovers() {
    let manager = Arc::new(BanManager::plugin(
        LAB,
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ));
    let site = Site {
        key: "ban-check",
        op: "ban-provider-check",
        directives: "bans",
        grants: &["ban-provider"],
        fallback: "unavailable",
        healthy: "not banned",
    };
    let checked = Arc::clone(&manager);
    let trigger = move |_: Arc<Lab>| {
        let manager = Arc::clone(&checked);
        boxed(async move {
            let ip: IpAddr = "203.0.113.7".parse().unwrap();
            match manager.check(&LoginAttempt::pre_auth(ip, "Steve")).await {
                Ok(None) => "not banned".to_owned(),
                Ok(Some(_)) => "banned".to_owned(),
                Err(_) => "unavailable".to_owned(),
            }
        })
    };
    let options = LabOptions {
        ban_manager: Some(manager),
        ..limits()
    };
    drive(&site, &trigger, options).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_permission_snapshot_gives_the_node_defaults_and_recovers() {
    let permissions = Arc::new(PermissionService::new_sync(&PermissionsConfig {
        provider: PermissionProviderSelection::Plugin(LAB.to_owned()),
        ..PermissionsConfig::default()
    }));
    let site = Site {
        key: "permission",
        op: "permission-snapshot-for",
        directives: "permissions",
        grants: &["permission-provider"],
        fallback: "Undefined",
        healthy: "True",
    };
    let asked = Arc::clone(&permissions);
    let trigger = move |_: Arc<Lab>| {
        let permissions = Arc::clone(&asked);
        boxed(async move {
            let checker = permissions.create_checker(&subject(3)).await;
            match permissions.value(checker.as_ref(), "fault-lab.answered") {
                Tristate::True => "True",
                Tristate::False => "False",
                Tristate::Undefined => "Undefined",
            }
            .to_owned()
        })
    };
    let options = LabOptions {
        permissions: Some(permissions),
        ..limits()
    };
    drive(&site, &trigger, options).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_the_first_on_enable_fails_the_enable_and_releases_what_it_registered() {
    for mode in Mode::TRAPS.into_iter().chain([Mode::Refuse]) {
        let logs = LogCapture::at(Level::INFO);
        async {
            let lab = Lab::stage(
                &[LabPlugin::lab(&format!(
                    "interval 50\nlisten lab-ping\nenable {}",
                    mode.as_str()
                ))],
                limits(),
            )
            .await;
            let plugin = lab.load(LAB).await;
            let ctx = lab.context(LAB);
            let started = Instant::now();
            let enabled = tokio::time::timeout(PROMPTLY, plugin.on_enable(ctx.as_ref()))
                .await
                .unwrap_or_else(|_| panic!("{mode:?}: the first on_enable hangs"));
            assert!(enabled.is_err(), "{mode:?}: the enable fails");
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "{mode:?}: bounded by the limits, took {:?}",
                started.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(lab.listeners(LAB), 0, "{mode:?}: its listeners are gone");
            assert_eq!(lab.tasks(LAB), 0, "{mode:?}: its tasks are cancelled");
            assert_eq!(
                fault_lab::count(&lab.log(LAB), "task"),
                0,
                "{mode:?}: no task of the failed instance ran"
            );
        }
        .with_subscriber(logs.clone())
        .await;
        assert!(
            logs.matching(RECOVERED).is_empty() && logs.matching("quarantined").is_empty(),
            "{mode:?}: a plugin that never enabled is not restarted: {:?}",
            logs.lines()
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_recovery_on_enable_counts_as_a_failed_restart_and_leaks_nothing() {
    for mode in Mode::TRAPS.into_iter().chain([Mode::Refuse]) {
        let logs = LogCapture::at(Level::INFO);
        async {
            let options = LabOptions {
                proxy_toml: "[wasm]\ncpu_budget = \"200ms\"\nmax_call_duration = \"1s\"\nmemory_limit_mb = 16\n\n[wasm.recovery]\nmax_restarts = 2\nbackoff_initial = \"400ms\"\n".to_owned(),
                ..LabOptions::default()
            };
            let lab = Lab::start(vec![LabPlugin::lab("interval 50\nlisten lab-ping")], options).await;
            let baseline = (lab.listeners(LAB), lab.tasks(LAB));
            lab.set_faults(
                LAB,
                &format!("interval 50\nlisten lab-ping\ncommand panic\nenable {}", mode.as_str()),
            );
            lab.dispatch("lab").await;
            assert_eq!(
                (lab.listeners(LAB), lab.tasks(LAB)),
                (0, 0),
                "{mode:?}: the quarantined plugin holds no listener and no task"
            );
            let enables = fault_lab::count_prefix(&lab.log(LAB), "enable recovered");
            assert_eq!(enables, 2, "{mode:?}: two restarts were tried: {:?}", lab.log(LAB));
            lab.set_faults(LAB, "interval 50\nlisten lab-ping");
            lab.wait_for(&format!("{mode:?}: the retry after the backoff"), || {
                (lab.listeners(LAB), lab.tasks(LAB)) == baseline
            })
            .await;
        }
        .with_subscriber(logs.clone())
        .await;
        let failed = logs.matching(FAILED);
        assert_eq!(failed.len(), 3, "{mode:?}: {failed:?}");
        assert!(
            failed[1..].iter().all(|line| line.contains("op=\"on-enable\"")),
            "{mode:?}: {failed:?}"
        );
        assert_eq!(logs.matching("quarantined").len(), 1, "{mode:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_on_disable_still_releases_the_plugin_in_bounded_time() {
    for mode in Mode::TRAPS.into_iter().chain([Mode::Refuse]) {
        let lab = Lab::start(
            vec![LabPlugin::lab("interval 50\nlisten lab-ping").grant("codec-filter")],
            limits(),
        )
        .await;
        assert!(!lab.codecs.owned_by(LAB).is_empty());
        lab.set_faults(LAB, &format!("interval 50\nlisten lab-ping\ndisable {}", mode.as_str()));
        let plugin = lab.take(LAB);
        let context = Arc::downgrade(&lab.context(LAB));
        let started = Instant::now();
        let disabled = tokio::time::timeout(PROMPTLY, plugin.on_disable())
            .await
            .unwrap_or_else(|_| panic!("{mode:?}: on_disable hangs"));
        let took = started.elapsed();
        assert!(disabled.is_err(), "{mode:?}: the failed on_disable is reported");
        assert!(took < Duration::from_secs(3), "{mode:?}: took {took:?}");
        assert_eq!(lab.listeners(LAB), 0, "{mode:?}");
        assert_eq!(lab.tasks(LAB), 0, "{mode:?}");
        assert!(lab.codecs.owned_by(LAB).is_empty(), "{mode:?}");
        lab.loader.unload(LAB).await.unwrap();
        drop(plugin);
        assert!(
            context.upgrade().is_none(),
            "{mode:?}: the plugin task ended and released the plugin context"
        );
    }
}

fn chain(lab: &Lab, connection: u64) -> CodecFilterChain {
    build_codec_chains(
        &lab.codecs,
        ProtocolVersion::new(767),
        connection,
        "127.0.0.1:1".parse().unwrap(),
        None,
    )
    .0
}

fn marked(chain: &mut CodecFilterChain) -> bool {
    let mut packet = RawPacket::new(faults::CODEC_MARK_PACKET, Bytes::from_static(b"original"));
    matches!(chain.process(&mut packet), FilterResult::Pass { .. }) && &packet.data[..] == b"fault-lab"
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fault_kind_in_a_codec_filter_only_disables_that_connection_side() {
    let logs = LogCapture::at(Level::INFO);
    async {
        let lab = Lab::start(vec![LabPlugin::lab("").grant("codec-filter")], limits()).await;
        let mut healthy = chain(&lab, 1);
        assert!(marked(&mut healthy));
        for mode in Mode::TRAPS {
            let started = Instant::now();
            let mut created = chain(&lab, faults::CODEC_FAULT_CONNECTION_BASE + mode.code());
            let create_took = started.elapsed();
            assert!(
                !marked(&mut created),
                "{mode:?}: a connection whose filter trapped in create passes packets through"
            );
            let mut filtered = chain(&lab, 2);
            assert!(marked(&mut filtered), "{mode:?}");
            let mut packet = RawPacket::new(
                faults::CODEC_FAULT_PACKET_BASE + i32::try_from(mode.code()).unwrap(),
                Bytes::from_static(b"x"),
            );
            let started = Instant::now();
            let verdict = filtered.process(&mut packet);
            let filter_took = started.elapsed();
            assert!(matches!(verdict, FilterResult::Pass { .. }), "{mode:?}");
            assert!(!marked(&mut filtered), "{mode:?}: the trapped instance passes from then on");
            assert!(marked(&mut healthy), "{mode:?}: other connections keep their filter");
            assert!(
                create_took < Duration::from_secs(2) && filter_took < Duration::from_secs(2),
                "{mode:?}: create {create_took:?}, filter {filter_took:?}"
            );
        }
        assert!(logs.matching(RECOVERED).is_empty(), "the main instance is untouched");
        lab.dispatch("lab").await;
    }
    .with_subscriber(logs.clone())
    .await;
    assert!(logs.matching(FAILED).is_empty(), "{:?}", logs.matching(FAILED));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_whose_metadata_traps_does_not_stop_the_others_from_being_discovered() {
    let tmp = tempfile::tempdir().unwrap();
    support::add_precompiled_fixture(tmp.path(), "fault-lab").await;
    support::add_fixture(tmp.path(), "metadata-trap", "metadata-trap");
    let loader = support::loader_from_toml(LIMITS);
    let discovered = tokio::time::timeout(PROMPTLY, loader.discover(tmp.path()))
        .await
        .expect("discovery returns");
    let ids: Vec<String> = discovered
        .as_ref()
        .map(|all| all.iter().map(|metadata| metadata.id.clone()).collect())
        .unwrap_or_default();
    assert!(
        ids.contains(&LAB.to_owned()),
        "the healthy plugin is discovered next to the one whose metadata traps: {discovered:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_whose_metadata_never_returns_does_not_hang_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    support::add_precompiled_fixture(tmp.path(), "fault-lab").await;
    support::add_fixture(tmp.path(), "metadata-sleep", "metadata-sleep");
    let loader = support::loader_from_toml(LIMITS);
    let started = Instant::now();
    let discovered = tokio::time::timeout(Duration::from_secs(20), loader.discover(tmp.path())).await;
    assert!(
        discovered.is_ok(),
        "discovery is still waiting on metadata() after {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_whose_metadata_traps_does_not_fail_the_plugin_manager_discovery() {
    let tmp = tempfile::tempdir().unwrap();
    support::add_precompiled_fixture(tmp.path(), "fault-lab").await;
    support::add_fixture(tmp.path(), "metadata-trap", "metadata-trap");
    let loader: Box<dyn PluginLoader> = Box::new(support::loader_from_toml(LIMITS));
    let mut manager = infrarust_core::plugin::manager::PluginManager::new(vec![loader]);
    let discovered = tokio::time::timeout(PROMPTLY, manager.discover_all(tmp.path()))
        .await
        .expect("discovery returns");
    assert!(
        discovered.is_ok(),
        "one plugin whose metadata() traps fails the whole discovery, so the proxy does not start: {:?}",
        discovered.err().map(|e| e.to_string().chars().take(200).collect::<String>())
    );
}
