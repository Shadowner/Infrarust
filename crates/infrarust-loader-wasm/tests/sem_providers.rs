#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::error::ServiceError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::lifecycle::{
    PermissionsSetupEvent, PermissionsSetupResult, PreLoginEvent, PreLoginResult,
};
use infrarust_api::loader::PluginLoader;
use infrarust_api::permissions::{PermissionMap, PermissionSubject, Tristate};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::ban_service::{BanRequest, BanTarget, LoginAttempt};
use infrarust_api::types::{Component, GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_config::{BanConfig, PermissionProviderSelection, PermissionsConfig};
use infrarust_core::ban::{BAN_CHECK_UNAVAILABLE, BanManager};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::permissions::PermissionService;
use infrarust_core::registry::ConnectionRegistry;
use infrarust_loader_wasm::WasmPluginLoader;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, read_log, stage};

const PROBE: &str = "sem-probe";

struct Probe {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    _plugin: Box<dyn Plugin>,
}

impl Probe {
    async fn start(config: &str, options: EnvOptions, before_load: impl FnOnce(&TestEnv)) -> Self {
        let (tmp, plugins_dir) = stage(PROBE);
        let data = plugins_dir.join(PROBE);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("probe.txt"), config).unwrap();
        let env = make_env_with(plugins_dir.clone(), options);
        before_load(&env);
        let loader = loader_from_toml("");
        loader.discover(&plugins_dir).await.unwrap();
        let plugin = load_enabled(&loader, &env.factory, PROBE).await;
        Self {
            _tmp: tmp,
            data,
            env,
            _loader: loader,
            _plugin: plugin,
        }
    }

    fn log(&self) -> Vec<String> {
        read_log(&self.data)
    }

    fn bus(&self) -> &dyn EventBus {
        &*self.env.event_bus
    }
}

async fn ban_manager(check_timeout: &str) -> Arc<BanManager> {
    let config: BanConfig = toml::from_str(&format!(
        "provider = \"{PROBE}\"\ncheck_timeout = \"{check_timeout}\"\n"
    ))
    .unwrap();
    Arc::new(
        BanManager::from_config(
            &config,
            Arc::new(ConnectionRegistry::new()),
            Arc::new(EventBusImpl::new()),
        )
        .await
        .unwrap(),
    )
}

fn with_bans(manager: &Arc<BanManager>) -> EnvOptions {
    EnvOptions {
        ban_service: Arc::clone(manager)
            as Arc<dyn infrarust_api::services::ban_service::BanService>,
        ban_manager: Some(Arc::clone(manager)),
        ..EnvOptions::default()
    }
    .grant(PROBE, "ban-provider")
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn login(name: &str) -> LoginAttempt {
    LoginAttempt::pre_auth(ip("203.0.113.7"), name)
}

async fn bans_probe(config: &str, check_timeout: &str) -> (Probe, Arc<BanManager>) {
    let manager = ban_manager(check_timeout).await;
    let probe = Probe::start(config, with_bans(&manager), |_| {}).await;
    (probe, manager)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_check_answering_an_error_refuses_the_login_and_answers_the_ping() {
    let (probe, manager) = bans_probe("bans", "5s").await;
    let refusal = manager
        .refuse(&login("Fail"))
        .await
        .expect("a check that errs refuses the login");
    assert_eq!(refusal.message.to_plain(), BAN_CHECK_UNAVAILABLE);
    assert!(
        manager
            .refuse(&LoginAttempt::status(ip("198.51.100.1")))
            .await
            .is_none(),
        "a status ping whose check errs is answered"
    );
    assert!(
        manager
            .refuse(&LoginAttempt::status(ip("198.51.100.2")))
            .await
            .is_none(),
        "a status ping whose check traps is answered"
    );
    assert!(manager.refuse(&login("Steve")).await.is_none());
    assert!(probe.log().contains(&"enable recovered 1".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn ban_calls_that_fail_or_trap_reach_the_caller_as_the_documented_errors() {
    let (_probe, manager) = bans_probe("bans", "5s").await;
    let failed = manager
        .issue(BanRequest::new(BanTarget::Username("FailBan".into())))
        .await;
    assert!(
        matches!(&failed, Err(ServiceError::OperationFailed(message)) if message.contains("refuses FailBan")),
        "{failed:?}"
    );
    let trapped = manager
        .issue(BanRequest::new(BanTarget::Username("TrapBan".into())))
        .await;
    assert!(
        matches!(trapped, Err(ServiceError::Unavailable(_))),
        "{trapped:?}"
    );
    let asked = manager.get(&BanTarget::Username("Fail".into())).await;
    assert!(
        matches!(asked, Err(ServiceError::OperationFailed(_))),
        "{asked:?}"
    );
    let issued = manager
        .issue(BanRequest::new(BanTarget::Username("Griefer".into())))
        .await
        .unwrap();
    assert_eq!(
        manager.refuse(&login("griefer")).await.map(|r| r.message.to_plain()),
        Some("probe: banned".to_owned()),
        "{issued:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_without_ip_ranges_never_receives_a_range_ban() {
    let (probe, manager) = bans_probe("bans noranges", "5s").await;
    assert!(!manager.features().ip_ranges);
    let refused = manager
        .issue(BanRequest::new(BanTarget::IpRange(
            "10.0.0.0/8".parse().unwrap(),
        )))
        .await;
    assert!(
        matches!(refused, Err(ServiceError::OperationFailed(_))),
        "{refused:?}"
    );
    assert!(
        !probe.log().iter().any(|line| line.starts_with("stored")),
        "{:?}",
        probe.log()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_ban_check_is_cut_at_check_timeout_and_does_not_refuse_the_logins_behind_it() {
    let (probe, manager) = bans_probe("bans", "300ms").await;
    let started = Instant::now();
    let slow = manager.refuse(&login("Sleep2000")).await;
    let cut = started.elapsed();
    assert!(slow.is_some(), "the slow login itself is refused");
    assert!(cut < Duration::from_millis(1000), "{cut:?}");

    let mut refused = Vec::new();
    for name in ["Steve", "Alex", "Notch", "Jeb"] {
        if manager.refuse(&login(name)).await.is_some() {
            refused.push(format!("{name} at {:?}", started.elapsed()));
        }
    }
    assert!(
        refused.is_empty(),
        "logins of unbanned players arriving while the provider still runs one slow check are refused \
         with \"{BAN_CHECK_UNAVAILABLE}\": {refused:?} (log {:?})",
        probe.log()
    );
}

fn permission_service() -> Arc<PermissionService> {
    Arc::new(PermissionService::new_sync(&PermissionsConfig {
        provider: PermissionProviderSelection::Plugin(PROBE.to_owned()),
        ..PermissionsConfig::default()
    }))
}

fn subject(id: u64, name: &str) -> PermissionSubject {
    PermissionSubject::player(
        PlayerId::new(id),
        GameProfile {
            uuid: uuid::Uuid::from_u128(u128::from(id)),
            username: name.to_owned(),
            properties: vec![],
        },
        true,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

async fn perms_probe(config: &str) -> (Probe, Arc<PermissionService>) {
    let permissions = permission_service();
    let probe = Probe::start(
        config,
        EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            ..EnvOptions::default()
        }
        .grant(PROBE, "permission-provider"),
        |_| {},
    )
    .await;
    (probe, permissions)
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_nodes_are_trimmed_and_lowercased_on_both_sides_of_the_boundary() {
    let (_probe, permissions) = perms_probe("perms").await;
    let mixed = permissions.create_checker(&subject(3, "Mixed")).await;
    assert_eq!(permissions.value(mixed.as_ref(), "warps.use"), Tristate::True);
    assert_eq!(permissions.value(mixed.as_ref(), "Warps.Use"), Tristate::True);
    assert_eq!(permissions.value(mixed.as_ref(), " WARPS.USE "), Tristate::True);
    assert_eq!(
        permissions.value(mixed.as_ref(), "warps.admin"),
        Tristate::False
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_snapshot_leaves_the_player_with_the_node_defaults_and_a_warning() {
    let logs = LogCapture::at(Level::WARN);
    let value = async {
        let (_probe, permissions) = perms_probe("perms").await;
        let big = permissions.create_checker(&subject(4, "Big")).await;
        permissions.value(big.as_ref(), "demo.use")
    }
    .with_subscriber(logs.clone())
    .await;
    assert_eq!(value, Tristate::Undefined);
    assert!(
        !logs.matching("gave no snapshot").is_empty(),
        "{:?}",
        logs.lines()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permission_provider_trapping_for_the_console_leaves_the_console_with_the_node_defaults() {
    let (probe, permissions) = perms_probe("perms console-trap").await;
    let console = permissions.console_checker().await;
    assert!(
        !console.has_permission("infrarust.command.kick"),
        "the console is a subject like any other: a failed answer gives it the node defaults"
    );
    assert!(probe.log().contains(&"snapshot console".to_owned()));
}

fn steve() -> Arc<dyn infrarust_api::player::Player> {
    support::session_player(
        1,
        support::nil_profile("Steve"),
        767,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_native_custom_checker_reaches_a_wasm_listener_as_the_snapshot_it_describes() {
    let probe = Probe::start("permsetup 192 record", EnvOptions::default(), |env| {
        let bus: &dyn EventBus = &*env.event_bus;
        bus.subscribe::<PermissionsSetupEvent, _>(EventPriority::EARLY, |event| {
            event.set_result(PermissionsSetupResult::Custom(Arc::new(
                PermissionMap::new().with("demo.use", true),
            )));
        });
    })
    .await;
    let event = probe
        .env
        .event_bus
        .fire(PermissionsSetupEvent::new(steve(), true))
        .await;
    let PermissionsSetupResult::Custom(checker) = event.result() else {
        panic!("the native checker stays in place");
    };
    assert!(checker.has_permission("demo.use"));
    assert_eq!(
        probe.log(),
        ["enable", "permsetup@192 saw custom admin=false rules=demo.use=true"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_snapshot_given_at_permissions_setup_is_what_a_later_native_listener_sees() {
    let seen: Arc<Mutex<Option<bool>>> = Arc::default();
    let observed = Arc::clone(&seen);
    let probe = Probe::start("permsetup 64 provide", EnvOptions::default(), move |env| {
        let bus: &dyn EventBus = &*env.event_bus;
        bus.subscribe::<PermissionsSetupEvent, _>(EventPriority::LATE, move |event| {
            if let PermissionsSetupResult::Custom(checker) = event.result() {
                *observed.lock().unwrap() = Some(checker.has_permission("probe.node"));
            }
        });
    })
    .await;
    probe
        .env
        .event_bus
        .fire(PermissionsSetupEvent::new(steve(), true))
        .await;
    assert_eq!(*seen.lock().unwrap(), Some(true));
    assert_eq!(probe.log(), ["enable", "permsetup@64 saw use-default"]);
}

fn native_append(bus: &dyn EventBus, priority: u8, tag: &'static str) {
    bus.subscribe::<ChatMessageEvent, _>(EventPriority::custom(priority), move |event| {
        let base = match event.result() {
            ChatMessageResult::Modify { message } => message.clone(),
            _ => event.message.clone(),
        };
        event.set_result(ChatMessageResult::Modify {
            message: format!("{base}|{tag}"),
        });
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn native_and_wasm_listeners_share_one_priority_order_over_one_event() {
    let probe = Probe::start(
        "chat-append 64 w64\nchat-append 128 w128\nchat-append 192 w192",
        EnvOptions::default().grant(PROBE, "chat-intercept"),
        |env| {
            native_append(&*env.event_bus, 0, "n0");
            native_append(&*env.event_bus, 128, "n128a");
        },
    )
    .await;
    native_append(probe.bus(), 128, "n128b");
    native_append(probe.bus(), 255, "n255");
    let event = probe
        .env
        .event_bus
        .fire(ChatMessageEvent::new(
            steve(),
            "hello".to_owned(),
            false,
            Some(ServerId::new("lobby")),
        ))
        .await;
    let ChatMessageResult::Modify { message } = event.result() else {
        panic!("every listener modified the message");
    };
    assert_eq!(message, "hello|n0|w64|n128a|w128|n128b|w192|n255");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wasm_listener_sees_and_can_undo_a_native_denial() {
    let final_seen: Arc<Mutex<Option<String>>> = Arc::default();
    let observed = Arc::clone(&final_seen);
    let probe = Probe::start(
        "prelogin 128 record\nprelogin 192 allow",
        EnvOptions::default(),
        |env| {
            let bus: &dyn EventBus = &*env.event_bus;
            bus.subscribe::<PreLoginEvent, _>(EventPriority::EARLY, |event| {
                event.set_result(PreLoginResult::Denied {
                    reason: Component::text("Banned"),
                });
            });
        },
    )
    .await;
    probe.bus().subscribe::<PreLoginEvent, _>(EventPriority::LAST, move |event| {
        *observed.lock().unwrap() = Some(format!("{:?}", event.result()));
    });
    let event = probe
        .env
        .event_bus
        .fire(PreLoginEvent::new(
            support::nil_profile("Steve"),
            "203.0.113.7:40000".parse().unwrap(),
            ProtocolVersion::new(767),
            "play.example.com".to_owned(),
        ))
        .await;
    assert!(matches!(event.result(), PreLoginResult::Allowed));
    assert_eq!(final_seen.lock().unwrap().as_deref(), Some("Allowed"));
    assert_eq!(
        probe.log(),
        [
            "enable",
            "prelogin@128 saw denied:Banned",
            "prelogin@192 saw denied:Banned"
        ]
    );
}
