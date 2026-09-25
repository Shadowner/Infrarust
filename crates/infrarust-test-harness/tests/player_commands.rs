#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use infrarust_api::event::BoxFuture;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler, SessionEndReason};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::packets::play::chat::SChatMessage;
use tokio::sync::Notify;

use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, FakeBackend, HarnessError, LoginBehavior, LoginOutcome,
    ProtocolVersion, ScriptedPlugin, ServerSpec, TestProxy, version_matrix,
};

const T: Duration = DEFAULT_TIMEOUT;
const KICK_ROUNDS: usize = 50;
const HOLD: &str = "hold";

type Ended = Arc<Mutex<Vec<SessionEndReason>>>;

struct HoldForever {
    ended: Ended,
}

impl LimboHandler for HoldForever {
    fn name(&self) -> &str {
        HOLD
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Hold })
    }

    fn on_session_end(&self, _player_id: PlayerId, reason: SessionEndReason) -> BoxFuture<'_, ()> {
        self.ended.lock().unwrap().push(reason);
        Box::pin(async {})
    }
}

fn holding_plugin(ended: &Ended) -> ScriptedPlugin {
    let ended = Arc::clone(ended);
    ScriptedPlugin::new("holder").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(HoldForever {
            ended: Arc::clone(&ended),
        }));
    })
}

struct SlowGate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl LimboHandler for SlowGate {
    fn name(&self) -> &str {
        HOLD
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            HandlerResult::Accept
        })
    }
}

fn held_hub() -> ServerSpec {
    ServerSpec::offline("hub")
        .unreachable()
        .limbo_handlers([HOLD])
        .network("main")
}

async fn kick_reason_is_always_delivered(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let mut lost = Vec::new();
    for round in 0..KICK_ROUNDS {
        let username = format!("Kicked{round}");
        let mut session = proxy
            .client(version)
            .login(&username)
            .await
            .unwrap()
            .joined()
            .unwrap();
        let conn = backend.next_connection(T).await.unwrap();
        let player = proxy.wait_for_player(&username, T).await.unwrap();

        tokio::time::timeout(T, player.disconnect(Component::text("bye")))
            .await
            .expect("disconnect must not block");

        match session.expect_disconnect(T).await {
            Ok(info) if info.state == ConnectionState::Play && info.text == "bye" => {}
            other => lost.push(format!("round {round}: {other:?}")),
        }
        conn.closed(T).await.unwrap();
        proxy.wait_for_connection_count(0, T).await.unwrap();
    }

    assert!(
        lost.is_empty(),
        "{} of {KICK_ROUNDS} kicks lost their reason: {lost:#?}",
        lost.len()
    );
    proxy.shutdown().await.unwrap();
}

version_matrix!(kick_reason_is_always_delivered; p47 = 47, p764 = 764, p774 = 774);

async fn limbo_message_arrives_while_held(version: ProtocolVersion) {
    let ended = Ended::default();
    let proxy = TestProxy::builder()
        .server(held_hub())
        .plugin(holding_plugin(&ended))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    player
        .send_message(Component::text("still waiting"))
        .unwrap();

    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        "still waiting"
    );
    assert!(
        ended.lock().unwrap().is_empty(),
        "the player left limbo before the message: {:?}",
        ended.lock().unwrap()
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_message_arrives_while_held; p47 = 47, p764 = 764, p774 = 774);

async fn limbo_kick_keeps_its_reason(version: ProtocolVersion) {
    let ended = Ended::default();
    let proxy = TestProxy::builder()
        .server(held_hub())
        .plugin(holding_plugin(&ended))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    tokio::time::timeout(T, player.disconnect(Component::text("bye from limbo")))
        .await
        .expect("disconnect must not block");

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, "bye from limbo", "{info:?}");
    proxy.wait_for_connection_count(0, T).await.unwrap();
    assert_eq!(*ended.lock().unwrap(), vec![SessionEndReason::Kicked]);

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_kick_keeps_its_reason; p47 = 47, p764 = 764, p774 = 774);

async fn kick_during_limbo_entry_keeps_its_reason(version: ProtocolVersion) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let gate = {
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        ScriptedPlugin::new("gate").on_enable(move |ctx| {
            ctx.register_limbo_handler(Box::new(SlowGate {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }));
        })
    };
    let proxy = TestProxy::builder()
        .server(held_hub())
        .plugin(gate)
        .start()
        .await
        .unwrap();

    let login = login_in_background(&proxy, version);
    tokio::time::timeout(T, entered.notified())
        .await
        .expect("the player must reach the limbo handler");
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    tokio::time::timeout(T, player.disconnect(Component::text("bye before limbo")))
        .await
        .expect("disconnect must not block");
    release.notify_one();

    let outcome = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap();
    let info = outcome.unwrap().disconnected().unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, "bye before limbo", "{info:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(kick_during_limbo_entry_keeps_its_reason; p47 = 47, p764 = 764, p774 = 774);

async fn limbo_switch_request_leaves_limbo(version: ProtocolVersion) {
    let ended = Ended::default();
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(held_hub())
        .server(
            ServerSpec::offline("game")
                .backend(backend.addr())
                .network("main"),
        )
        .plugin(holding_plugin(&ended))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client_for("hub", version)
        .unwrap()
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    player.switch_server(ServerId::new("game")).await.unwrap();

    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), "Steve");
    session.expect_join(T).await.unwrap();
    session.chat("hello game").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "hello game"
    );
    assert_eq!(player.current_server(), Some(ServerId::new("game")));
    assert_eq!(*ended.lock().unwrap(), vec![SessionEndReason::Redirected]);

    proxy.shutdown().await.unwrap();
}

version_matrix!(limbo_switch_request_leaves_limbo; p47 = 47, p764 = 764, p774 = 774);

fn login_in_background(
    proxy: &TestProxy,
    version: ProtocolVersion,
) -> tokio::task::JoinHandle<Result<LoginOutcome, HarnessError>> {
    let client = proxy.client(version);
    tokio::spawn(async move { client.login("Steve").await })
}

async fn login_phase_kick_uses_login_disconnect(version: ProtocolVersion) {
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Hang)
        .spawn()
        .await
        .unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let login = login_in_background(&proxy, version);
    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Login);
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    tokio::time::timeout(T, player.disconnect(Component::text("Login bye")))
        .await
        .expect("disconnect must not block");

    let outcome = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap();
    let info = outcome.unwrap().disconnected().unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "Login bye", "{info:?}");
    conn.closed(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(login_phase_kick_uses_login_disconnect; p47 = 47, p764 = 764, p774 = 774);

async fn config_phase_kick_uses_config_disconnect(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let login = login_in_background(&proxy, version);
    let conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Config);
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    tokio::time::timeout(T, player.disconnect(Component::text("Config bye")))
        .await
        .expect("disconnect must not block");

    let outcome = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap();
    let info = outcome.unwrap().disconnected().unwrap();
    assert_eq!(info.state, ConnectionState::Config, "{info:?}");
    assert_eq!(info.text, "Config bye", "{info:?}");
    conn.closed(T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(config_phase_kick_uses_config_disconnect; p764 = 764, p774 = 774);

async fn config_phase_message_waits_for_join(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let login = login_in_background(&proxy, version);
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Config);
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    player.send_message(Component::text("hello")).unwrap();
    conn.finish_config(T).await.unwrap();

    let mut session = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "hello");

    session.chat("still connected").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "still connected"
    );
    conn.send_system_message_json(r#"{"text":"welcome"}"#)
        .await
        .unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "welcome");

    proxy.shutdown().await.unwrap();
}

version_matrix!(config_phase_message_waits_for_join; p764 = 764, p774 = 774);

async fn passthrough_disconnect_closes_the_connection(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::passthrough("pipe").backend(backend.addr()))
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    assert!(!player.is_active());

    tokio::time::timeout(T, player.disconnect(Component::text("unused")))
        .await
        .expect("disconnect must not block");

    match session.expect_disconnect(T).await {
        Err(HarnessError::Closed(_)) => {}
        other => panic!("expected the connection to close, got {other:?}"),
    }
    conn.closed(T).await.unwrap();
    proxy.wait_for_connection_count(0, T).await.unwrap();

    proxy.shutdown().await.unwrap();
}

version_matrix!(passthrough_disconnect_closes_the_connection; p47 = 47, p764 = 764, p774 = 774);
