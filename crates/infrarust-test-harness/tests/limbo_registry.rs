#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::limbo::{HANDLER_UNAVAILABLE, LimboHandlerError, LimboHandlerRegistration};
use infrarust_test_harness::{
    ClientSession, DEFAULT_TIMEOUT, FakeBackend, ProtocolVersion, ScriptedPlugin, ServerSpec,
    TestProxy,
};
use tokio::sync::mpsc;

const T: Duration = DEFAULT_TIMEOUT;
const GATE: &str = "late_gate";
const VERSION: ProtocolVersion = ProtocolVersion::V1_21;

type Holds = mpsc::UnboundedReceiver<SessionHandle>;

struct Gate {
    held: mpsc::UnboundedSender<SessionHandle>,
}

impl LimboHandler for Gate {
    fn name(&self) -> &str {
        GATE
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let _ = self.held.send(session.handle());
        Box::pin(async { HandlerResult::Hold })
    }
}

fn gate() -> (Box<Gate>, Holds) {
    let (held, holds) = mpsc::unbounded_channel();
    (Box::new(Gate { held }), holds)
}

async fn next_hold(holds: &mut Holds) -> SessionHandle {
    tokio::time::timeout(T, holds.recv())
        .await
        .expect("the player must reach the limbo handler")
        .expect("the limbo handler must stay registered")
}

async fn gated_proxy(backend: &FakeBackend, plugins: Vec<ScriptedPlugin>) -> TestProxy {
    plugins
        .into_iter()
        .fold(
            TestProxy::builder().server(
                ServerSpec::offline("lobby")
                    .backend(backend.addr())
                    .limbo_handlers([GATE]),
            ),
            |builder, plugin| builder.plugin(plugin),
        )
        .start()
        .await
        .unwrap()
}

async fn join(proxy: &TestProxy, username: &str) -> ClientSession {
    proxy
        .client(VERSION)
        .login(username)
        .await
        .unwrap()
        .joined()
        .unwrap()
}

#[tokio::test]
async fn a_handler_registered_after_startup_holds_the_player() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = gated_proxy(&backend, vec![ScriptedPlugin::new("late")]).await;

    let (handler, mut holds) = gate();
    let ctx = proxy.plugin_context("late").await.unwrap();
    ctx.register_limbo_handler(handler)
        .expect("the limbo handler registers");

    let mut session = join(&proxy, "Steve").await;
    let handle = next_hold(&mut holds).await;

    handle.complete(HandlerResult::Accept);
    let _conn = backend.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabling_the_plugin_releases_the_players_its_handler_holds() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (handler, mut holds) = gate();
    let handler = Mutex::new(Some(handler));
    let gatekeeper = ScriptedPlugin::new("gatekeeper").on_enable(move |ctx| {
        let handler = handler.lock().unwrap().take().unwrap();
        ctx.register_limbo_handler(handler)
            .expect("the limbo handler registers");
    });
    let proxy = gated_proxy(&backend, vec![gatekeeper]).await;

    let mut steve = join(&proxy, "Steve").await;
    next_hold(&mut holds).await;

    proxy.disable_plugin("gatekeeper").await.unwrap();

    let kicked = steve.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, HANDLER_UNAVAILABLE, "{kicked:?}");

    let _alex = join(&proxy, "Alex").await;
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Alex", T).await.unwrap();
    assert!(
        holds.try_recv().is_err(),
        "the removed handler saw no one else"
    );
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn unregistering_through_the_handle_releases_the_held_player() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = gated_proxy(&backend, vec![ScriptedPlugin::new("owner")]).await;
    let (handler, mut holds) = gate();
    let registration: LimboHandlerRegistration = proxy
        .plugin_context("owner")
        .await
        .unwrap()
        .register_limbo_handler(handler)
        .unwrap();
    assert_eq!(registration.name(), GATE);

    let mut steve = join(&proxy, "Steve").await;
    next_hold(&mut holds).await;

    assert!(registration.unregister());
    assert!(!registration.unregister());
    let kicked = steve.expect_disconnect(T).await.unwrap();
    assert_eq!(kicked.text, HANDLER_UNAVAILABLE, "{kicked:?}");
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_plugin_cannot_take_a_registered_name() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let refused: Arc<Mutex<Option<Result<(), LimboHandlerError>>>> = Arc::default();
    let first = ScriptedPlugin::new("first").on_enable(|ctx| {
        ctx.register_limbo_handler(gate().0)
            .expect("the first registration wins");
    });
    let slot = Arc::clone(&refused);
    let second = ScriptedPlugin::new("second")
        .after("first")
        .on_enable(move |ctx| {
            let outcome = ctx.register_limbo_handler(gate().0).map(|_| ());
            *slot.lock().unwrap() = Some(outcome);
        });
    let proxy = gated_proxy(&backend, vec![first, second]).await;

    assert_eq!(
        refused.lock().unwrap().take().unwrap(),
        Err(LimboHandlerError::NameTaken {
            name: GATE.to_string(),
            owner: "first".to_string(),
        })
    );
    proxy.shutdown().await.unwrap();
}

async fn a_server_listing_an_unregistered_handler_sends_the_player_on(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = gated_proxy(&backend, vec![]).await;

    let _session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _conn = backend.next_connection(T).await.unwrap();
    proxy.wait_for_player("Steve", T).await.unwrap();
    proxy.shutdown().await.unwrap();
}

infrarust_test_harness::version_matrix!(a_server_listing_an_unregistered_handler_sends_the_player_on; p47 = 47, p763 = 763, p764 = 764, p774 = 774);
