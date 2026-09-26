#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::error::ServiceError;
use infrarust_api::permissions::{PermissionSubject, Tristate};
use infrarust_api::player::Player;
use infrarust_api::plugin::Plugin;
use infrarust_api::services::ban_service::{
    BanQuery, BanRequest, BanSource, BanTarget, LoginAttempt, UnbanRequest,
};
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{GameProfile, PlayerId, ProtocolVersion, ServerId};
use infrarust_config::{BanProviderSelection, PermissionProviderSelection, PermissionsConfig};
use infrarust_core::ban::{BAN_CHECK_UNAVAILABLE, BanManager};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::permissions::PermissionService;
use infrarust_core::player::PlayerSession;
use infrarust_core::registry::ConnectionRegistry;
use infrarust_core::services::command_manager::DispatchOutcome;
use infrarust_loader_wasm::WasmPluginLoader;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use tracing::instrument::WithSubscriber;

use support::log_capture::LogCapture;
use support::{EnvOptions, TestEnv, load_enabled, loader_from_toml, make_env_with, stage};

const PROVIDER: &str = "provider";
const PROMPTLY: Duration = Duration::from_secs(10);
const QUARANTINE_AT_ONCE: &str =
    "[plugins.provider.wasm.recovery]\nmax_restarts = 0\nbackoff_initial = \"30s\"\n";
const PAST_THE_BACKOFF: Duration = Duration::from_secs(31);
const CPU_BUDGET: Duration = Duration::from_secs(2);
const SHORT_DEADLINE: &str =
    "[events]\nhandler_timeout = \"300ms\"\n\n[plugins.provider.wasm]\ncpu_budget = \"2s\"\n";

struct Fixture {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    env: TestEnv,
    _loader: WasmPluginLoader,
    _plugin: Box<dyn Plugin>,
}

impl Fixture {
    fn log(&self) -> Vec<String> {
        read_log(&self.data)
    }

    async fn dispatch(&self, line: &str) {
        let outcome = tokio::time::timeout(
            PROMPTLY,
            self.env.command_manager.dispatch(support::console(), line),
        )
        .await
        .expect("a command returns promptly");
        assert_eq!(outcome, DispatchOutcome::Executed, "{line}");
    }
}

fn read_log(data: &Path) -> Vec<String> {
    std::fs::read_to_string(data.join("log.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

struct Setup {
    config: &'static str,
    grants: &'static [&'static str],
    proxy_toml: &'static str,
    options: EnvOptions,
}

async fn start(setup: Setup) -> Fixture {
    let (tmp, plugins_dir) = stage(PROVIDER);
    let data = plugins_dir.join(PROVIDER);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("provider.txt"), setup.config).unwrap();
    let mut options = setup.options;
    for grant in setup.grants {
        options = options.grant(PROVIDER, grant);
    }
    let env = make_env_with(plugins_dir.clone(), options);
    let loader = loader_from_toml(setup.proxy_toml);
    infrarust_api::loader::PluginLoader::discover(&loader, &plugins_dir)
        .await
        .unwrap();
    let plugin = load_enabled(&loader, &env.factory, PROVIDER).await;
    Fixture {
        _tmp: tmp,
        data,
        env,
        _loader: loader,
        _plugin: plugin,
    }
}

fn ban_manager(selected: &str) -> Arc<BanManager> {
    Arc::new(BanManager::plugin(
        selected,
        Arc::new(ConnectionRegistry::new()),
        Arc::new(EventBusImpl::new()),
    ))
}

fn with_bans(manager: &Arc<BanManager>) -> EnvOptions {
    EnvOptions {
        ban_service: Arc::clone(manager)
            as Arc<dyn infrarust_api::services::ban_service::BanService>,
        ban_providers: Some(Arc::clone(manager)),
        ..EnvOptions::default()
    }
}

fn permission_service(provider: PermissionProviderSelection) -> Arc<PermissionService> {
    Arc::new(PermissionService::new_sync(&PermissionsConfig {
        provider,
        ..PermissionsConfig::default()
    }))
}

fn select(id: &str) -> PermissionProviderSelection {
    PermissionProviderSelection::Plugin(id.to_owned())
}

fn ip() -> IpAddr {
    "203.0.113.7".parse().unwrap()
}

fn login(name: &str) -> LoginAttempt {
    LoginAttempt::pre_auth(ip(), name)
}

fn profile(name: &str) -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::from_u128(u128::from(name.len() as u64)),
        username: name.to_owned(),
        properties: vec![],
    }
}

fn subject(id: u64, name: &str) -> PermissionSubject {
    PermissionSubject::player(
        PlayerId::new(id),
        profile(name),
        true,
        "203.0.113.7:40000".parse().unwrap(),
    )
}

async fn let_the_backoff_pass() {
    tokio::time::pause();
    tokio::time::advance(PAST_THE_BACKOFF).await;
    tokio::time::resume();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selected_wasm_ban_provider_answers_checks_bans_and_lists() {
    let manager = ban_manager(PROVIDER);
    let fx = start(Setup {
        config: "bans\nban Griefer griefing",
        grants: &["ban-provider"],
        proxy_toml: "",
        options: with_bans(&manager),
    })
    .await;

    let verdict = manager
        .check(&login("Griefer"))
        .await
        .unwrap()
        .expect("the provider bans the griefer");
    assert_eq!(verdict.kick_message.to_plain(), "provider: griefing");
    assert_eq!(verdict.entry.id, "seed-Griefer");
    assert_eq!(verdict.entry.source, BanSource::Console);
    assert_eq!(manager.check(&login("Steve")).await.unwrap(), None);
    assert!(manager.features().ip_ranges);

    let issued = manager
        .issue(
            BanRequest::new(BanTarget::Username("Alex".into()))
                .reason("spam")
                .source(BanSource::Plugin("moderation".into())),
        )
        .await
        .unwrap();
    assert_eq!(issued.entry.id, "p1");
    assert_eq!(issued.entry.source, BanSource::Plugin("moderation".into()));
    assert_eq!(issued.entry.reason.as_deref(), Some("spam"));
    let alex = BanTarget::Username("Alex".into());
    assert_eq!(manager.get(&alex).await.unwrap().unwrap().id, "p1");
    assert_eq!(
        manager.list(BanQuery::new()).await.unwrap().entries.len(),
        2
    );
    let revoked = manager
        .revoke(UnbanRequest::new(alex.clone()))
        .await
        .unwrap();
    assert_eq!(revoked.map(|entry| entry.id).as_deref(), Some("p1"));
    assert_eq!(manager.get(&alex).await.unwrap(), None);
    assert_eq!(fx.log(), ["enable", "bans ok"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_provider_registration_obeys_the_selection_and_the_capability() {
    let other = ban_manager("other");
    let refused = start(Setup {
        config: "bans",
        grants: &["ban-provider"],
        proxy_toml: "",
        options: with_bans(&other),
    })
    .await;
    assert_eq!(refused.log(), ["enable", "bans conflict"]);
    assert!(matches!(
        other.check(&login("Steve")).await,
        Err(ServiceError::Unavailable(_))
    ));

    let selected = ban_manager(PROVIDER);
    let ungranted = start(Setup {
        config: "bans",
        grants: &[],
        proxy_toml: "",
        options: with_bans(&selected),
    })
    .await;
    assert_eq!(ungranted.log(), ["enable", "bans permission-denied"]);
    assert!(
        selected.check(&login("Steve")).await.is_err(),
        "without a registered provider logins fail closed"
    );
    assert!(matches!(
        selected.selection(),
        BanProviderSelection::Plugin(id) if id == PROVIDER
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_provider_calling_the_ban_service_from_its_own_calls_is_refused_at_once() {
    let manager = ban_manager(PROVIDER);
    let fx = start(Setup {
        config: "bans\ncheck-service\nban Griefer griefing",
        grants: &["ban-provider", "ban"],
        proxy_toml: "",
        options: with_bans(&manager),
    })
    .await;

    fx.dispatch("pban Steve").await;
    let started = Instant::now();
    let verdict = tokio::time::timeout(PROMPTLY, manager.check(&login("Griefer")))
        .await
        .expect("a re-entrant check does not wait for its own deadline")
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
    assert!(
        verdict.is_some(),
        "the provider still answers its own check"
    );
    assert_eq!(
        fx.log(),
        [
            "enable",
            "bans ok",
            "pban Steve unavailable",
            "check service unavailable",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trapping_ban_check_refuses_the_login_and_the_recovered_instance_answers_again() {
    let manager = ban_manager(PROVIDER);
    let fx = start(Setup {
        config: "bans\nban Griefer griefing",
        grants: &["ban-provider"],
        proxy_toml: "",
        options: with_bans(&manager),
    })
    .await;

    let refusal = manager
        .refuse(&login("Trap"))
        .await
        .expect("a provider that traps refuses the login");
    assert_eq!(refusal.message.to_plain(), BAN_CHECK_UNAVAILABLE);
    assert_eq!(manager.check(&login("Steve")).await.unwrap(), None);
    assert!(manager.check(&login("Griefer")).await.unwrap().is_some());
    assert_eq!(
        fx.log(),
        ["enable", "bans ok", "enable recovered 1", "bans ok"],
        "the recovered instance registers again and the host keeps one registration"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_check_that_outlives_its_deadline_fails_closed_promptly() {
    let manager = ban_manager(PROVIDER);
    let fx = start(Setup {
        config: "bans",
        grants: &["ban-provider"],
        proxy_toml: SHORT_DEADLINE,
        options: with_bans(&manager),
    })
    .await;

    let started = Instant::now();
    let answer = tokio::time::timeout(PROMPTLY, manager.check(&login("Spin")))
        .await
        .expect("the adapter gives up at the call deadline");
    assert!(
        started.elapsed() < CPU_BUDGET,
        "the call deadline cut it off before the guest trapped: {:?}",
        started.elapsed()
    );
    match answer {
        Err(ServiceError::Unavailable(message)) => {
            assert!(message.contains("before the call's deadline"), "{message}");
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    fx.dispatch("pping").await;
    assert_eq!(
        fx.log(),
        [
            "enable",
            "bans ok",
            "enable recovered 1",
            "bans ok",
            "pping"
        ],
        "the spinning call ran out of CPU, and the next call reached a fresh instance"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_selected_wasm_permission_provider_answers_snapshots() {
    let permissions = permission_service(select(PROVIDER));
    let _fx = start(Setup {
        config: "permissions\ngrant Steve demo.use true\ngrant Steve demo.*.kick false\nconsole-admin",
        grants: &["permission-provider"],
        proxy_toml: "",
        options: EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            ..EnvOptions::default()
        },
    })
    .await;

    let steve = permissions.create_checker(&subject(1, "Steve")).await;
    assert_eq!(
        permissions.value(steve.as_ref(), "demo.use"),
        Tristate::True
    );
    assert_eq!(
        permissions.value(steve.as_ref(), "demo.mod.kick"),
        Tristate::Undefined,
        "a wildcard only closes a trailing segment"
    );
    let alex = permissions.create_checker(&subject(2, "Alex")).await;
    assert_eq!(
        permissions.value(alex.as_ref(), "demo.use"),
        Tristate::Undefined
    );
    let console = permissions.console_checker().await;
    assert!(console.has_permission("infrarust.command.kick"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permission_provider_registration_obeys_the_selection_and_the_capability() {
    let builtin = permission_service(PermissionProviderSelection::Builtin);
    let refused = start(Setup {
        config: "permissions",
        grants: &["permission-provider"],
        proxy_toml: "",
        options: EnvOptions {
            permissions: Some(builtin),
            ..EnvOptions::default()
        },
    })
    .await;
    assert_eq!(refused.log()[1], "permissions conflict");

    let selected = permission_service(select(PROVIDER));
    let steve = session(1, "Steve", &selected);
    let ungranted = start(Setup {
        config: "permissions",
        grants: &[],
        proxy_toml: "",
        options: EnvOptions {
            permissions: Some(Arc::clone(&selected)),
            player_registry: Arc::new(Online {
                players: vec![Arc::clone(&steve)],
            }),
            ..EnvOptions::default()
        },
    })
    .await;
    ungranted.dispatch("pset Steve demo.use true").await;
    ungranted.dispatch("prelease Steve").await;
    assert_eq!(
        ungranted.log(),
        [
            "enable",
            "permissions permission-denied",
            "pset Steve permission-denied",
            "prelease Steve permission-denied",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_permission_provider_leaves_the_subject_with_the_node_defaults_and_a_warning() {
    let logs = LogCapture::at(Level::WARN);
    let permissions = permission_service(select(PROVIDER));
    async {
        let _fx = start(Setup {
            config: "permissions\ngrant Steve demo.use true\ngrant Trap demo.use true",
            grants: &["permission-provider"],
            proxy_toml: "",
            options: EnvOptions {
                permissions: Some(Arc::clone(&permissions)),
                ..EnvOptions::default()
            },
        })
        .await;

        let trapped = permissions.create_checker(&subject(9, "Trap")).await;
        assert_eq!(
            permissions.value(trapped.as_ref(), "demo.use"),
            Tristate::Undefined
        );
        let steve = permissions.create_checker(&subject(1, "Steve")).await;
        assert_eq!(
            permissions.value(steve.as_ref(), "demo.use"),
            Tristate::True
        );
    }
    .with_subscriber(logs.clone())
    .await;

    let warned = logs.matching("gave no snapshot");
    assert_eq!(warned.len(), 1, "{:?}", logs.lines());
    assert!(
        warned[0].contains("plugin=\"provider\"") || warned[0].contains("plugin=provider"),
        "{warned:?}"
    );
    assert!(warned[0].contains("subject=\"Trap\""), "{warned:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_permission_answer_that_outlives_its_deadline_gets_the_node_defaults_promptly() {
    let permissions = permission_service(select(PROVIDER));
    let fx = start(Setup {
        config: "permissions\ngrant Spin demo.use true",
        grants: &["permission-provider"],
        proxy_toml: SHORT_DEADLINE,
        options: EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            ..EnvOptions::default()
        },
    })
    .await;

    let started = Instant::now();
    let checker = tokio::time::timeout(PROMPTLY, permissions.create_checker(&subject(5, "Spin")))
        .await
        .expect("the adapter gives up at the call deadline");
    assert!(started.elapsed() < CPU_BUDGET, "{:?}", started.elapsed());
    assert_eq!(
        permissions.value(checker.as_ref(), "demo.use"),
        Tristate::Undefined
    );
    fx.dispatch("pping").await;
    assert_eq!(
        fx.log(),
        [
            "enable",
            "permissions ok",
            "enable recovered 1",
            "permissions ok",
            "pping"
        ]
    );
}

struct Online {
    players: Vec<Arc<PlayerSession>>,
}

impl infrarust_api::services::player_registry::private::Sealed for Online {}

impl PlayerRegistry for Online {
    fn get_player(&self, username: &str) -> Option<Arc<dyn Player>> {
        self.players
            .iter()
            .find(|player| player.profile().username == username)
            .map(|player| Arc::clone(player) as Arc<dyn Player>)
    }
    fn get_player_by_uuid(&self, uuid: &uuid::Uuid) -> Option<Arc<dyn Player>> {
        self.players
            .iter()
            .find(|player| player.profile().uuid == *uuid)
            .map(|player| Arc::clone(player) as Arc<dyn Player>)
    }
    fn get_player_by_id(&self, id: PlayerId) -> Option<Arc<dyn Player>> {
        self.players
            .iter()
            .find(|player| player.id() == id)
            .map(|player| Arc::clone(player) as Arc<dyn Player>)
    }
    fn get_players_by_ip(&self, _ip: IpAddr) -> Vec<Arc<dyn Player>> {
        Vec::new()
    }
    fn get_players_on_server(&self, _server: &ServerId) -> Vec<Arc<dyn Player>> {
        Vec::new()
    }
    fn get_all_players(&self) -> Vec<Arc<dyn Player>> {
        Vec::new()
    }
    fn online_count(&self) -> usize {
        self.players.len()
    }
    fn online_count_on(&self, _server: &ServerId) -> usize {
        0
    }
}

fn session(id: u64, name: &str, permissions: &Arc<PermissionService>) -> Arc<PlayerSession> {
    let (commands, _) = PlayerSession::channel();
    Arc::new(
        PlayerSession::new(
            PlayerId::new(id),
            profile(name),
            ProtocolVersion::MINECRAFT_1_21,
            "203.0.113.7:40000".parse().unwrap(),
            None,
            true,
            true,
            commands,
            CancellationToken::new(),
            infrarust_core::permissions::default_checker(),
            Arc::new(infrarust_core::loadbalancer::BackendLoad::new()),
        )
        .with_permissions(Arc::clone(permissions)),
    )
}

async fn set_up_permissions(permissions: &PermissionService, player: &PlayerSession) {
    let checker = permissions
        .create_checker(&player.permission_subject())
        .await;
    player.set_permission_checker(checker);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_snapshot_updates_a_player_live_and_the_snapshot_survives_a_recovery() {
    let permissions = permission_service(select(PROVIDER));
    let steve = session(1, "Steve", &permissions);
    let alex = session(2, "Alex", &permissions);
    let fx = start(Setup {
        config: "permissions",
        grants: &["permission-provider"],
        proxy_toml: "",
        options: EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            player_registry: Arc::new(Online {
                players: vec![Arc::clone(&steve), Arc::clone(&alex)],
            }),
            ..EnvOptions::default()
        },
    })
    .await;
    set_up_permissions(&permissions, &steve).await;
    assert!(!steve.has_permission("demo.use"));
    let changes = steve.subscribe_permissions();

    fx.dispatch("pset Steve demo.use true").await;
    assert!(steve.has_permission("demo.use"), "the update is live");
    assert!(
        changes.has_changed().unwrap(),
        "the player's command tree is refreshed"
    );

    fx.dispatch("pset Alex demo.use true").await;
    assert!(!alex.has_permission("demo.use"));

    fx.dispatch("ptrap").await;
    assert!(
        steve.has_permission("demo.use"),
        "the host keeps the snapshot while the guest recovers"
    );
    fx.dispatch("pset Steve demo.kick true").await;
    assert!(steve.has_permission("demo.kick"));
    assert!(
        !steve.has_permission("demo.use"),
        "the recovered guest starts from its own state"
    );

    fx.dispatch("prelease Steve").await;
    assert!(
        !steve.has_permission("demo.kick"),
        "a released player is cleared"
    );
    fx.dispatch("pset Steve demo.use true").await;
    assert!(!steve.has_permission("demo.use"));

    assert_eq!(
        fx.log(),
        [
            "enable",
            "permissions ok",
            "pset Steve ok",
            "pset Alex not-found",
            "enable recovered 1",
            "permissions ok",
            "pset Steve ok",
            "prelease Steve ok",
            "pset Steve not-found",
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn quarantined_providers_fail_closed_for_bans_and_safe_for_permissions_until_recovered() {
    let manager = ban_manager(PROVIDER);
    let permissions = permission_service(select(PROVIDER));
    let fx = start(Setup {
        config: "bans\npermissions\ngrant Steve demo.use true",
        grants: &["ban-provider", "permission-provider"],
        proxy_toml: QUARANTINE_AT_ONCE,
        options: EnvOptions {
            permissions: Some(Arc::clone(&permissions)),
            ..with_bans(&manager)
        },
    })
    .await;
    assert_eq!(manager.check(&login("Steve")).await.unwrap(), None);

    fx.dispatch("ptrap").await;
    assert!(matches!(
        manager.check(&login("Steve")).await,
        Err(ServiceError::Unavailable(_))
    ));
    let quarantined = permissions.create_checker(&subject(1, "Steve")).await;
    assert_eq!(
        permissions.value(quarantined.as_ref(), "demo.use"),
        Tristate::Undefined
    );

    let_the_backoff_pass().await;
    assert_eq!(manager.check(&login("Steve")).await.unwrap(), None);
    let recovered = permissions.create_checker(&subject(1, "Steve")).await;
    assert_eq!(
        permissions.value(recovered.as_ref(), "demo.use"),
        Tristate::True
    );
    assert_eq!(
        fx.log(),
        [
            "enable",
            "bans ok",
            "permissions ok",
            "enable recovered 1",
            "bans ok",
            "permissions ok",
        ]
    );
}
