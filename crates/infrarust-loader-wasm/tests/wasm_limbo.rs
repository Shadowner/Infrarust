#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use infrarust_api::limbo::test_util::RecordingLimboSession;
use infrarust_api::limbo::{HandlerResult, LimboEntryContext, LimboHandler};
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::Plugin;
use infrarust_api::types::{GameProfile, PlayerId, ServerId};
use infrarust_config::ProxyConfig;
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::context::PluginContextImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use infrarust_loader_wasm::{WasmPluginLoader, build_engine};

mod mock_services;
use mock_services::{MockBanService, MockConfigService, MockPlayerRegistry};

const FIXTURE_DIR: &str = env!("INFRARUST_WASM_FIXTURE_DIR");
const FIXTURE: &str = "limbo-handler";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

fn fresh_loader() -> WasmPluginLoader {
    let config: ProxyConfig = toml::from_str("").expect("default proxy config");
    WasmPluginLoader::new(build_engine(&config).expect("build engine"))
}

fn stage(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    std::fs::copy(
        fixture_path(fixture),
        plugins_dir.join(format!("{fixture}.wasm")),
    )
    .unwrap();
    (tmp, plugins_dir)
}

fn limbo_env(plugins_dir: PathBuf, plugin_id: &str, grant: bool) -> PluginContextFactoryImpl {
    let services = PluginServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(MockPlayerRegistry),
        server_manager: Arc::new(NoopServerManager),
        ban_service: Arc::new(MockBanService),
        command_manager: Arc::new(CommandManagerImpl::new()),
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: Arc::new(MockConfigService),
        plugin_registry: Arc::new(PluginRegistryImpl::new()),
        codec_filter_registry: Arc::new(CodecFilterRegistryImpl::new()),
        transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
        domain_router: Arc::new(DomainRouter::new()),
        proxy_shutdown: tokio_util::sync::CancellationToken::new(),
        proxy_info: infrarust_api::services::proxy_info::ProxyInfo::default(),
        plugins_dir,
    };
    let mut configs = HashMap::new();
    if grant {
        configs.insert(
            plugin_id.to_string(),
            PluginPermissions {
                permissions: vec!["limbo".to_string()],
                trusted: false,
            },
        );
    }
    PluginContextFactoryImpl::new(services, configs)
}

async fn load_enabled(
    loader: &WasmPluginLoader,
    factory: &PluginContextFactoryImpl,
    id: &str,
) -> Box<dyn Plugin> {
    let plugin = loader
        .load(id, factory)
        .await
        .unwrap_or_else(|e| panic!("load {id}: {e}"));
    let ctx = factory.create_context(id);
    plugin
        .on_enable(ctx.as_ref())
        .await
        .unwrap_or_else(|e| panic!("enable {id}: {e}"));
    plugin
}

fn take_handlers(factory: &PluginContextFactoryImpl, id: &str) -> Vec<Box<dyn LimboHandler>> {
    factory
        .create_context(id)
        .as_any()
        .downcast_ref::<PluginContextImpl>()
        .expect("PluginContextImpl")
        .take_limbo_handlers()
}

fn find_handler(handlers: Vec<Box<dyn LimboHandler>>, name: &str) -> Box<dyn LimboHandler> {
    handlers
        .into_iter()
        .find(|h| h.name() == name)
        .unwrap_or_else(|| panic!("handler {name} not registered"))
}

fn test_session(player_id: u64) -> Arc<RecordingLimboSession> {
    let profile = GameProfile {
        uuid: uuid::Uuid::nil(),
        username: "Tester".to_string(),
        properties: Vec::new(),
    };
    RecordingLimboSession::new(
        PlayerId::new(player_id),
        profile,
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn registers_named_handlers_and_holds_on_entry() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;

    let handlers = take_handlers(&factory, FIXTURE);
    assert_eq!(
        handlers.len(),
        4,
        "the guest registered `gate`, `boom`, `timed-gate`, `delayed-gate`"
    );
    let gate = find_handler(handlers, "gate");

    let session = test_session(1);
    let outcome = gate.on_player_enter(session.as_ref()).await;
    assert!(
        matches!(outcome, HandlerResult::Hold),
        "the gate holds the player on entry"
    );
    assert_eq!(
        session.messages().len(),
        1,
        "the entry greeting reached the native session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_completes_the_hold_with_accept() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let gate = find_handler(take_handlers(&factory, FIXTURE), "gate");

    let session = test_session(7);
    let _ = gate.on_player_enter(session.as_ref()).await;
    gate.on_command(session.as_ref(), "continue", &[]).await;

    let completions = session.completions();
    assert_eq!(completions.len(), 1, "`/continue` completed the hold once");
    assert!(
        matches!(completions[0], HandlerResult::Accept),
        "`/continue` accepts the player"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_can_redirect() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let gate = find_handler(take_handlers(&factory, FIXTURE), "gate");

    let session = test_session(8);
    let _ = gate.on_player_enter(session.as_ref()).await;
    gate.on_command(session.as_ref(), "redirect", &[]).await;

    let completions = session.completions();
    assert!(
        matches!(completions.last(), Some(HandlerResult::Redirect(server)) if server.as_str() == "hub"),
        "`/redirect` redirects to the named server, got {completions:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_is_dispatched_and_disconnect_cleans_up() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let gate = find_handler(take_handlers(&factory, FIXTURE), "gate");

    let session = test_session(9);
    let _ = gate.on_player_enter(session.as_ref()).await;
    gate.on_chat(session.as_ref(), "hello there").await;
    assert!(
        session.messages().len() >= 2,
        "the chat handler replied via the session"
    );

    gate.on_disconnect(PlayerId::new(9)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_noops_without_capability() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, false);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;

    assert!(
        take_handlers(&factory, FIXTURE).is_empty(),
        "without the Limbo capability the guest registers no handlers"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn trapped_entry_handler_fails_closed() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let boom = find_handler(take_handlers(&factory, FIXTURE), "boom");

    let session = test_session(13);
    let outcome = boom.on_player_enter(session.as_ref()).await;
    assert!(
        matches!(outcome, HandlerResult::Deny(_)),
        "a guest trap in on_player_enter denies the player (fail-closed)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timed_gate_holds_with_timeout_then_command_accepts() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let gate = find_handler(take_handlers(&factory, FIXTURE), "timed-gate");

    let session = test_session(21);
    let outcome = gate.on_player_enter(session.as_ref()).await;
    match outcome {
        HandlerResult::HoldWithTimeout { after, on_timeout } => {
            assert_eq!(
                after,
                std::time::Duration::from_secs(5),
                "the guest's 5s deadline survives the round-trip"
            );
            assert!(
                matches!(*on_timeout, HandlerResult::Deny(_)),
                "the timeout outcome is a terminal Deny, got {on_timeout:?}"
            );
        }
        other => panic!("expected HoldWithTimeout, got {other:?}"),
    }

    gate.on_command(session.as_ref(), "continue", &[]).await;
    let completions = session.completions();
    assert!(
        matches!(completions.last(), Some(HandlerResult::Accept)),
        "`/continue` releases the timed hold, got {completions:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_gate_completes_from_scheduled_task() {
    let (_tmp, dir) = stage(FIXTURE);
    let loader = fresh_loader();
    let factory = limbo_env(dir.clone(), FIXTURE, true);
    loader.discover(&dir).await.unwrap();
    let _plugin = load_enabled(&loader, &factory, FIXTURE).await;
    let gate = find_handler(take_handlers(&factory, FIXTURE), "delayed-gate");

    let session = test_session(34);
    // The guest stores `session.handle()`, schedules a 50ms delay, and returns Hold.
    let outcome = gate.on_player_enter(session.as_ref()).await;
    assert!(
        matches!(outcome, HandlerResult::Hold),
        "the delayed gate holds while it waits for its timer, got {outcome:?}"
    );
    assert!(
        session.completions().is_empty(),
        "nothing is completed synchronously during on_player_enter"
    );

    let mut released = false;
    for _ in 0..200 {
        if matches!(session.completions().last(), Some(HandlerResult::Accept)) {
            released = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        released,
        "the scheduled task completed the held player via the stored handle, got {:?}",
        session.completions()
    );
}
