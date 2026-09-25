#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::error::ServiceError;
use infrarust_api::event::BoxFuture;
use infrarust_api::services::ban_service::{
    BanEntry, BanFeatures, BanPage, BanProvider, BanProviderRejected, BanQuery, BanRequest,
    BanService, BanSource, BanTarget, BanVerdict, LoginAttempt, LoginStage, UnbanRequest,
};
use infrarust_api::types::Component;
use infrarust_core::auth::game_profile::offline_uuid;
use infrarust_core::ban::storage::BanStorage;
use infrarust_core::ban::{BAN_CHECK_UNAVAILABLE, FileBanStorage};
use infrarust_core::console::ConsoleServices;
use infrarust_core::console::commands::register_all;
use infrarust_core::console::dispatcher::CommandDispatcher;
use infrarust_core::console::output::CommandOutput;
use infrarust_core::services::config_service::ConfigServiceImpl;
use infrarust_test_harness::{
    BackendConn, ClientSession, ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend,
    ProtocolVersion, Recorder, ScriptedPlugin, ServerSpec, TestProxy,
};
use serde_json::json;
use toml::{Table, Value};

const T: Duration = DEFAULT_TIMEOUT;
const VERSION: ProtocolVersion = ProtocolVersion(774);
const GUARD: &str = "guard";

#[derive(Default)]
struct GuardProvider {
    entries: Mutex<Vec<BanEntry>>,
    attempts: Mutex<Vec<LoginAttempt>>,
    stuck: bool,
}

impl GuardProvider {
    fn with(entries: impl IntoIterator<Item = (BanTarget, &'static str)>) -> Arc<Self> {
        let entries = entries
            .into_iter()
            .enumerate()
            .map(|(n, (target, reason))| {
                BanEntry::new(format!("seed{n}"), target, BanSource::Console).reason(reason)
            })
            .collect();
        Arc::new(Self {
            entries: Mutex::new(entries),
            attempts: Mutex::default(),
            stuck: false,
        })
    }

    fn stuck() -> Arc<Self> {
        Arc::new(Self {
            stuck: true,
            ..Self::default()
        })
    }

    fn attempts(&self) -> Vec<LoginAttempt> {
        self.attempts.lock().unwrap().clone()
    }

    fn applies(entry: &BanEntry, attempt: &LoginAttempt) -> bool {
        let needs_profile = matches!(entry.target, BanTarget::Uuid(_));
        entry.target.matches(attempt) && (!needs_profile || attempt.stage == LoginStage::PostAuth)
    }
}

fn guard_message(entry: &BanEntry) -> String {
    format!("guard: {}", entry.reason.as_deref().unwrap_or("banned"))
}

impl BanProvider for GuardProvider {
    fn check<'a>(
        &'a self,
        attempt: &'a LoginAttempt,
    ) -> BoxFuture<'a, Result<Option<BanVerdict>, ServiceError>> {
        self.attempts.lock().unwrap().push(attempt.clone());
        if self.stuck {
            return Box::pin(std::future::pending());
        }
        let verdict = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| Self::applies(entry, attempt))
            .map(|entry| {
                BanVerdict::new(entry.clone()).message(Component::text(guard_message(entry)))
            });
        Box::pin(async move { Ok(verdict) })
    }

    fn ban(&self, request: BanRequest) -> BoxFuture<'_, Result<BanEntry, ServiceError>> {
        let mut entries = self.entries.lock().unwrap();
        let mut entry = BanEntry::new(
            format!("g{}", entries.len()),
            request.target,
            request.source.unwrap_or(BanSource::System),
        );
        entry.reason = request.reason;
        entries.push(entry.clone());
        Box::pin(async move { Ok(entry) })
    }

    fn unban(
        &self,
        request: UnbanRequest,
    ) -> BoxFuture<'_, Result<Option<BanEntry>, ServiceError>> {
        let mut entries = self.entries.lock().unwrap();
        let removed = entries
            .iter()
            .position(|entry| entry.target == request.target)
            .map(|index| entries.remove(index));
        Box::pin(async move { Ok(removed) })
    }

    fn get<'a>(
        &'a self,
        target: &'a BanTarget,
    ) -> BoxFuture<'a, Result<Option<BanEntry>, ServiceError>> {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| &entry.target == target)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list(&self, _query: BanQuery) -> BoxFuture<'_, Result<BanPage, ServiceError>> {
        let entries = self.entries.lock().unwrap().clone();
        Box::pin(async move { Ok(BanPage::new(entries, None)) })
    }

    fn features(&self) -> BanFeatures {
        BanFeatures::new().ip_ranges(true)
    }
}

type ServiceSlot = Arc<Mutex<Option<Arc<dyn BanService>>>>;
type RegistrationSlot = Arc<Mutex<Option<Result<(), BanProviderRejected>>>>;

struct Guard {
    service: ServiceSlot,
    registration: RegistrationSlot,
}

impl Guard {
    fn service(&self) -> Arc<dyn BanService> {
        self.service
            .lock()
            .unwrap()
            .clone()
            .expect("the guard plugin was enabled")
    }

    fn registration(&self) -> Option<Result<(), BanProviderRejected>> {
        self.registration.lock().unwrap().clone()
    }
}

fn guard_plugin(provider: &Arc<GuardProvider>) -> (ScriptedPlugin, Guard) {
    let guard = Guard {
        service: ServiceSlot::default(),
        registration: RegistrationSlot::default(),
    };
    let service = Arc::clone(&guard.service);
    let registration = Arc::clone(&guard.registration);
    let provider = Arc::clone(provider);
    let plugin = ScriptedPlugin::new(GUARD).on_enable(move |ctx| {
        let provider = Arc::clone(&provider) as Arc<dyn BanProvider>;
        *registration.lock().unwrap() = Some(ctx.register_ban_provider(provider));
        *service.lock().unwrap() = Some(ctx.ban_service_handle());
    });
    (plugin, guard)
}

fn ban_table(table: &mut Table) -> &mut Table {
    table
        .get_mut("ban")
        .and_then(Value::as_table_mut)
        .expect("the harness writes a [ban] table")
}

fn select_provider(table: &mut Table, provider: &str) {
    ban_table(table).insert("provider".into(), Value::String(provider.into()));
}

fn use_ban_file(table: &mut Table, file: &Path) {
    ban_table(table).insert(
        "file".into(),
        Value::String(file.to_str().unwrap().to_string()),
    );
}

fn receive_proxy_protocol(table: &mut Table) {
    table.insert("receive_proxy_protocol".into(), Value::Boolean(true));
}

fn from(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip.parse().unwrap(), port)
}

async fn builtin_ban_file(dir: &Path, target: BanTarget, reason: &str) -> std::path::PathBuf {
    let path = dir.join("bans.json");
    let storage = FileBanStorage::new(path.clone());
    storage
        .add_ban(BanEntry::new(String::new(), target, BanSource::Console).reason(reason))
        .await
        .unwrap();
    path
}

struct Joined {
    session: ClientSession,
    _backend: BackendConn,
}

async fn join(
    proxy: &TestProxy,
    backend: &FakeBackend,
    username: &str,
    source: Option<SocketAddr>,
) -> Joined {
    let client = proxy.client(VERSION);
    let client = match source {
        Some(source) => client.proxy_protocol(source),
        None => client,
    };
    let session = client.login(username).await.unwrap().joined().unwrap();
    let backend = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player(username, T).await.unwrap();
    Joined {
        session,
        _backend: backend,
    }
}

fn console(proxy: &TestProxy) -> ConsoleServices {
    let services = proxy.services();
    let running = proxy.running().expect("the proxy is running");
    ConsoleServices::new(
        Arc::clone(&services.player_registry),
        Arc::clone(&services.connection_registry),
        Arc::clone(&services.ban_manager),
        services.server_manager.clone(),
        Arc::new(ConfigServiceImpl::new(
            Arc::clone(&services.domain_router),
            services.config_path.clone(),
            Arc::clone(&services.config),
        )),
        Arc::clone(running.plugin_manager()),
        Arc::clone(&services.permission_service),
        Arc::clone(&services.command_manager),
        proxy.shutdown_token().clone(),
        running.start_time(),
    )
}

async fn run_console(proxy: &TestProxy, line: &str) {
    let services = console(proxy);
    let mut dispatcher = CommandDispatcher::new();
    register_all(&mut dispatcher);
    if let CommandOutput::Error(message) = dispatcher.dispatch(line, &services).await {
        panic!("{line}: {message}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_selected_plugin_refuses_an_address_before_authentication() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let provider =
        GuardProvider::with([(BanTarget::Ip("203.0.113.7".parse().unwrap()), "address")]);
    let (plugin, guard) = guard_plugin(&provider);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| {
            receive_proxy_protocol(table);
            select_provider(table, GUARD);
        })
        .start()
        .await
        .unwrap();
    assert_eq!(guard.registration(), Some(Ok(())));

    let info = proxy
        .client(VERSION)
        .proxy_protocol(from("203.0.113.7", 50001))
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "guard: address");
    let refused = provider.attempts().pop().expect("the provider was asked");
    assert_eq!(refused.stage, LoginStage::PreAuth);
    assert_eq!(refused.ip, "203.0.113.7".parse::<IpAddr>().unwrap());
    assert_eq!(refused.username.as_deref(), Some("Mallory"));
    assert_eq!(refused.virtual_host.as_deref(), proxy.domain("lobby"));
    assert_eq!(recorder.count(EventKind::PreLogin), 0);
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert!(
        proxy
            .client(VERSION)
            .proxy_protocol(from("203.0.113.7", 50002))
            .status()
            .await
            .is_err(),
        "a refused address gets no status answer"
    );

    let alice = join(&proxy, &backend, "Alice", Some(from("198.51.100.1", 50003))).await;
    alice.session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_selected_plugin_refuses_a_profile_after_authentication() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let provider = GuardProvider::with([(BanTarget::Uuid(offline_uuid("Banned")), "account")]);
    let (plugin, _guard) = guard_plugin(&provider);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| select_provider(table, GUARD))
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(VERSION)
        .login("Banned")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "guard: account");
    let stages: Vec<LoginStage> = provider.attempts().iter().map(|a| a.stage).collect();
    assert_eq!(stages, [LoginStage::PreAuth, LoginStage::PostAuth]);
    let refused = provider.attempts().pop().unwrap();
    assert_eq!(refused.uuid, Some(offline_uuid("Banned")));
    assert!(!refused.uuid_verified, "an offline profile is not verified");
    assert_eq!(refused.server.as_ref().map(|s| s.as_str()), Some("lobby"));
    assert_eq!(recorder.count(EventKind::PreLogin), 1);
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(proxy.connection_count(), 0);

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_that_never_answers_refuses_logins_and_answers_pings() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let provider = GuardProvider::stuck();
    let (plugin, _guard) = guard_plugin(&provider);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| {
            select_provider(table, GUARD);
            ban_table(table).insert("check_timeout".into(), Value::String("200ms".into()));
        })
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, BAN_CHECK_UNAVAILABLE);
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    let status = proxy.client(VERSION).status().await.unwrap();
    assert_eq!(
        status.json["description"]["text"],
        json!("Infrarust fake backend")
    );
    let stages: Vec<LoginStage> = provider.attempts().iter().map(|a| a.stage).collect();
    assert!(stages.contains(&LoginStage::Status), "{stages:?}");

    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_builtin_list_is_not_consulted_while_a_plugin_provides_bans() {
    let dir = tempfile::tempdir().unwrap();
    let file = builtin_ban_file(dir.path(), BanTarget::Username("Steve".into()), "builtin").await;
    let backend = FakeBackend::builder().spawn().await.unwrap();

    let builtin_file = file.clone();
    let builtin = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .patch_config(move |table| use_ban_file(table, &builtin_file))
        .start()
        .await
        .unwrap();
    let info = builtin
        .client(VERSION)
        .login("Steve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert!(info.text.contains("builtin"), "{info:?}");
    builtin.shutdown().await.unwrap();

    let provider = GuardProvider::with([]);
    let (plugin, _guard) = guard_plugin(&provider);
    let delegated = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .patch_config(move |table| {
            use_ban_file(table, &file);
            select_provider(table, GUARD);
        })
        .start()
        .await
        .unwrap();

    let steve = join(&delegated, &backend, "Steve", None).await;
    assert!(!provider.attempts().is_empty());

    steve.session.quit().await;
    delegated.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_provider_lets_everyone_in() {
    let dir = tempfile::tempdir().unwrap();
    let file = builtin_ban_file(dir.path(), BanTarget::Username("Steve".into()), "builtin").await;
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let provider = GuardProvider::with([(BanTarget::Username("Steve".into()), "guard")]);
    let (plugin, guard) = guard_plugin(&provider);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .patch_config(move |table| {
            use_ban_file(table, &file);
            select_provider(table, "none");
        })
        .start()
        .await
        .unwrap();

    assert_eq!(
        guard.registration(),
        Some(Err(BanProviderRejected::NotSelected {
            selected: "none".into()
        }))
    );
    let steve = join(&proxy, &backend, "Steve", None).await;
    assert!(provider.attempts().is_empty());
    let refused = guard
        .service()
        .ban(BanRequest::new(BanTarget::Username("Steve".into())))
        .await;
    assert!(
        matches!(refused, Err(ServiceError::Unavailable(_))),
        "{refused:?}"
    );

    steve.session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_ban_kicks_the_player_and_announces_who_issued_it() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let provider = GuardProvider::with([]);
    let (plugin, guard) = guard_plugin(&provider);
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(plugin)
        .plugin(recorder.plugin())
        .patch_config(|table| select_provider(table, GUARD))
        .start()
        .await
        .unwrap();
    let mut steve = join(&proxy, &backend, "Steve", None).await;
    let service = guard.service();
    let target = BanTarget::Username("Steve".into());

    let entry = service
        .ban(BanRequest::new(target.clone()).reason("spam"))
        .await
        .unwrap();

    assert_eq!(entry.source, BanSource::Plugin(GUARD.into()));
    let info = steve.session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, "guard: spam");
    let issued = recorder
        .wait_for(|e| e.kind == EventKind::BanIssued, T)
        .await
        .unwrap();
    assert_eq!(issued.detail["id"], json!(entry.id));
    assert_eq!(issued.detail["source"], json!("plugin:guard"));
    assert_eq!(issued.detail["target"], json!("username:Steve"));
    assert_eq!(issued.detail["silent"], json!(false));
    let disconnect = recorder
        .wait_for(|e| e.kind == EventKind::Disconnect, T)
        .await
        .unwrap();
    assert_eq!(disconnect.detail["cause"], json!("kicked"));
    assert_eq!(disconnect.detail["reason"], json!("guard: spam"));

    let removed = service
        .unban(UnbanRequest::new(target))
        .await
        .unwrap()
        .expect("the ban existed");
    assert_eq!(removed.id, entry.id);
    let revoked = recorder
        .wait_for(|e| e.kind == EventKind::BanRevoked, T)
        .await
        .unwrap();
    assert_eq!(revoked.detail["id"], json!(entry.id));
    assert_eq!(revoked.detail["source"], json!("plugin:guard"));
    let again = join(&proxy, &backend, "Steve", None).await;

    again.session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_console_range_ban_kicks_every_player_behind_the_range() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .patch_config(receive_proxy_protocol)
        .start()
        .await
        .unwrap();
    let mut inside = Vec::new();
    for (username, ip) in [
        ("Alice", "203.0.113.7"),
        ("Bob", "203.0.113.8"),
        ("Carol", "203.0.113.200"),
    ] {
        inside.push(join(&proxy, &backend, username, Some(from(ip, 50000))).await);
    }
    let outside = join(&proxy, &backend, "Dave", Some(from("198.51.100.9", 50000))).await;

    run_console(&proxy, "ban-ip 203.0.113.0/24 botnet").await;

    for joined in &mut inside {
        let info = joined.session.expect_disconnect(T).await.unwrap();
        assert_eq!(info.state, ConnectionState::Play, "{info:?}");
        assert!(info.text.contains("botnet"), "{info:?}");
    }
    proxy.wait_for_connection_count(1, T).await.unwrap();
    assert!(proxy.wait_for_player("Dave", T).await.is_ok());
    let issued = recorder
        .wait_for(|e| e.kind == EventKind::BanIssued, T)
        .await
        .unwrap();
    assert_eq!(issued.detail["source"], json!("console"));
    assert_eq!(issued.detail["target"], json!("range:203.0.113.0/24"));
    let refused = proxy
        .client(VERSION)
        .proxy_protocol(from("203.0.113.99", 50001))
        .login("Eve")
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert!(refused.text.contains("botnet"), "{refused:?}");

    outside.session.quit().await;
    proxy.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn console_bans_are_attributed_to_the_console() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let target = BanTarget::Username("Griefer".into());

    run_console(&proxy, "ban Griefer 1h grief").await;
    let entry = proxy
        .services()
        .ban_manager
        .get(&target)
        .await
        .unwrap()
        .expect("the console ban is stored");
    assert_eq!(entry.source, BanSource::Console);
    assert!(!entry.is_permanent());
    run_console(&proxy, "unban Griefer").await;

    let issued = recorder
        .wait_for(|e| e.kind == EventKind::BanIssued, T)
        .await
        .unwrap();
    assert_eq!(issued.detail["source"], json!("console"));
    assert_eq!(issued.detail["id"], json!(entry.id));
    let revoked = recorder
        .wait_for(|e| e.kind == EventKind::BanRevoked, T)
        .await
        .unwrap();
    assert_eq!(revoked.detail["source"], json!("console"));
    assert!(
        proxy
            .services()
            .ban_manager
            .get(&target)
            .await
            .unwrap()
            .is_none()
    );

    proxy.shutdown().await.unwrap();
}
