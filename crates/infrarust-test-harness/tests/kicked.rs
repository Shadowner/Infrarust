#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::event::{BoxFuture, EventPriority, ResultedEvent};
use infrarust_api::events::connection::{KickedFromServerEvent, KickedFromServerResult};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handle::SessionHandle;
use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::player::Player;
use infrarust_api::types::{Component, NamedColor, ServerId};
use infrarust_protocol::packets::play::chat::SChatMessage;
use infrarust_protocol::packets::play::start_configuration::SAcknowledgeConfiguration;
use infrarust_test_harness::plugin_message::{self, PluginMessage};
use infrarust_test_harness::recorder::component_value;
use infrarust_test_harness::{
    ClientSession, ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, FakeSessionServer,
    LoginBehavior, PacketFrame, ProtocolVersion, Recorded, Recorder, ScriptedPlugin, ServerSpec,
    TestProxy, version_matrix, wire,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

const T: Duration = DEFAULT_TIMEOUT;
const STEVE: &str = "Steve";
const CATCH: &str = "catch";
const ACKED: &str = "harness:acked";
const KICK_JSON: &str = r#"{ "color" : "red", "translate" : "disconnect.closed", "extra" : [ { "text" : " (maintenance)" } ] }"#;

fn kick_component() -> Component {
    Component::translatable("disconnect.closed")
        .color(NamedColor::Red)
        .append(Component::text(" (maintenance)"))
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn nbt_string(out: &mut Vec<u8>, name: &str, value: &str) {
    out.push(0x08);
    put_string(out, name);
    put_string(out, value);
}

fn kick_payload(version: ProtocolVersion) -> Vec<u8> {
    if version.less_than(ProtocolVersion::V1_20_3) {
        return KICK_JSON.as_bytes().to_vec();
    }
    let mut out = vec![0x0A];
    nbt_string(&mut out, "color", "red");
    nbt_string(&mut out, "translate", "disconnect.closed");
    out.push(0x09);
    put_string(&mut out, "extra");
    out.push(0x0A);
    out.extend_from_slice(&1i32.to_be_bytes());
    nbt_string(&mut out, "text", " (maintenance)");
    out.push(0x00);
    out.push(0x00);
    out
}

fn text_value(text: &str) -> Value {
    component_value(&Component::text(text))
}

fn named(event: &Recorded, username: &str) -> bool {
    event.username.as_deref() == Some(username)
}

fn network(spec: ServerSpec) -> ServerSpec {
    spec.network("main")
}

fn napping(spec: ServerSpec, message: &'static str) -> ServerSpec {
    spec.unreachable().patch(move |table| {
        table.insert(
            "disconnect_message".into(),
            toml::Value::String(message.into()),
        );
    })
}

async fn sync(session: &mut ClientSession, player: &dyn Player) {
    player.send_message(Component::text("sync")).unwrap();
    assert_eq!(session.expect_system_text(T).await.unwrap(), "sync");
}

async fn kicked(recorder: &Recorder, server: &str) -> Recorded {
    recorder
        .wait_for(
            |e| e.kind == EventKind::KickedFromServer && e.detail["server"] == json!(server),
            T,
        )
        .await
        .unwrap()
}

async fn disconnected(recorder: &Recorder) -> Recorded {
    recorder
        .wait_for(|e| e.kind == EventKind::Disconnect && named(e, STEVE), T)
        .await
        .unwrap()
}

fn on_kick(
    id: &str,
    decide: impl Fn(&mut KickedFromServerEvent) + Send + Sync + 'static,
) -> ScriptedPlugin {
    ScriptedPlugin::new(id).on::<KickedFromServerEvent>(EventPriority::NORMAL, decide)
}

type Held = mpsc::UnboundedReceiver<(SessionHandle, LimboEntryContext)>;

struct Catch {
    held: mpsc::UnboundedSender<(SessionHandle, LimboEntryContext)>,
}

impl LimboHandler for Catch {
    fn name(&self) -> &str {
        CATCH
    }

    fn on_player_enter<'a>(
        &'a self,
        session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let _ = self
            .held
            .send((session.handle(), session.entry_context().clone()));
        Box::pin(async { HandlerResult::Hold })
    }
}

fn catcher() -> (ScriptedPlugin, Held) {
    let (held, holds) = mpsc::unbounded_channel();
    let plugin = ScriptedPlugin::new("catcher").on_enable(move |ctx| {
        ctx.register_limbo_handler(Box::new(Catch { held: held.clone() }))
            .expect("the limbo handler registers");
    });
    (plugin, holds)
}

async fn next_hold(holds: &mut Held) -> (SessionHandle, LimboEntryContext) {
    tokio::time::timeout(T, holds.recv())
        .await
        .expect("the player must reach the limbo handler")
        .expect("the limbo handler must stay registered")
}

async fn a_play_kick_reaches_the_client_as_sent(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    recorder
        .wait_for_kind(EventKind::ServerPostConnect, T)
        .await
        .unwrap();

    conn.kick_raw(kick_payload(version)).await.unwrap();

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.raw, kick_payload(version), "{info:?}");
    let expected = component_value(&kick_component());
    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["reason"], expected, "{kick:?}");
    assert_eq!(kick.detail["cause"], json!("play_disconnect"), "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(false), "{kick:?}");
    assert_eq!(kick.detail["previous_server"], json!(null), "{kick:?}");
    assert_eq!(
        kick.detail["result"],
        json!({ "disconnect_player": null }),
        "{kick:?}"
    );
    let disconnect = disconnected(&recorder).await;
    assert_eq!(disconnect.detail["cause"], json!("backend_closed"));
    assert_eq!(disconnect.detail["reason_json"], expected, "{disconnect:?}");
    assert_eq!(disconnect.detail["last_server"], json!("lobby"));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, a_play_kick_reaches_the_client_as_sent);

async fn a_play_kick_redirect_joins_the_target(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let redirect = on_kick("redirect", |e| {
        if e.server == ServerId::new("a") {
            e.redirect_to(ServerId::new("b"));
        }
    });
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(redirect)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    recorder
        .wait_for_kind(EventKind::ServerPostConnect, T)
        .await
        .unwrap();

    conn_a.kick_json(r#"{"text":"Restarting"}"#).await.unwrap();

    session.expect_join(T).await.unwrap();
    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;
    assert_eq!(player.current_server(), Some(ServerId::new("b")));

    let kick = kicked(&recorder, "a").await;
    assert_eq!(kick.detail["reason"], text_value("Restarting"));
    assert_eq!(kick.detail["result"], json!({ "redirect_to": "b" }));
    let pre_connects =
        recorder.filter(|e| e.kind == EventKind::ServerPreConnect && e.seq > kick.seq);
    assert_eq!(pre_connects.len(), 1, "{pre_connects:?}");
    assert_eq!(pre_connects[0].detail["server"], json!("b"));
    assert_eq!(pre_connects[0].detail["cause"], json!("kick_redirect"));
    assert_eq!(pre_connects[0].detail["previous_server"], json!("a"));

    session.chat("hello b").await.unwrap();
    assert_eq!(
        conn_b.expect::<SChatMessage>(T).await.unwrap().message,
        "hello b"
    );

    conn_b.kick_json(r#"{"text":"B closes"}"#).await.unwrap();
    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, "B closes", "{info:?}");
    let kick = kicked(&recorder, "b").await;
    assert_eq!(kick.detail["during_connect"], json!(false), "{kick:?}");
    assert_eq!(kick.detail["previous_server"], json!("a"), "{kick:?}");
    assert_eq!(kick.detail["current_server"], json!("b"), "{kick:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, a_play_kick_redirect_joins_the_target);

async fn a_play_kick_can_park_the_player_in_limbo(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let (catch, mut holds) = catcher();
    let to_limbo = on_kick("to_limbo", |e| {
        e.set_result(KickedFromServerResult::SendToLimbo {
            limbo_handlers: vec![CATCH.to_string()],
        });
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(catch)
        .plugin(to_limbo)
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut first = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    first.kick_json(r#"{"text":"Crashed"}"#).await.unwrap();

    let (handle, context) = next_hold(&mut holds).await;
    match context {
        LimboEntryContext::KickedFromServer { server, reason } => {
            assert_eq!(server, ServerId::new("lobby"));
            assert_eq!(reason, Component::text("Crashed"));
        }
        other => panic!("expected a kick context, got {other:?}"),
    }
    sync(&mut session, player.as_ref()).await;

    handle.complete(HandlerResult::Accept);
    let _second = backend.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    sync(&mut session, player.as_ref()).await;
    assert_eq!(player.current_server(), Some(ServerId::new("lobby")));

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, a_play_kick_can_park_the_player_in_limbo);

async fn notify_after_a_play_kick_disconnects_with_the_message(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let notify = on_kick("notify", |e| {
        e.set_result(KickedFromServerResult::Notify {
            message: Component::text("Your server went away"),
        });
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(notify)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    conn.kick_json(r#"{"text":"Bye"}"#).await.unwrap();

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, "Your server went away", "{info:?}");
    let disconnect = disconnected(&recorder).await;
    assert_eq!(disconnect.detail["cause"], json!("backend_closed"));
    assert_eq!(
        disconnect.detail["reason_json"],
        text_value("Your server went away")
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, notify_after_a_play_kick_disconnects_with_the_message);

struct FailedSwitch {
    message: &'static str,
    cause: &'static str,
    reason: Value,
}

async fn assert_failed_switch_keeps_the_player(
    version: ProtocolVersion,
    target: ServerSpec,
    target_backend: Option<&FakeBackend>,
    expected: FailedSwitch,
) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(target))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    recorder
        .wait_for_kind(EventKind::ServerPostConnect, T)
        .await
        .unwrap();

    player.switch_server(ServerId::new("b")).await.unwrap();
    let _refused = match target_backend {
        Some(backend) => {
            let mut conn = backend.next_connection(T).await.unwrap();
            if conn.state() == ConnectionState::Config {
                conn.kick_json(r#"{"text":"B turned you away"}"#)
                    .await
                    .unwrap();
            }
            Some(conn)
        }
        None => None,
    };

    assert_eq!(
        session.expect_system_text(T).await.unwrap(),
        expected.message
    );
    let kick = kicked(&recorder, "b").await;
    assert_eq!(kick.detail["cause"], json!(expected.cause), "{kick:?}");
    assert_eq!(kick.detail["reason"], expected.reason, "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(kick.detail["previous_server"], json!("a"), "{kick:?}");
    assert_eq!(
        kick.detail["result"],
        json!({ "notify": expected.message }),
        "{kick:?}"
    );
    assert_eq!(player.current_server(), Some(ServerId::new("a")));
    session.chat("still on a").await.unwrap();
    assert_eq!(
        conn_a.expect::<SChatMessage>(T).await.unwrap().message,
        "still on a"
    );
    assert_eq!(recorder.count(EventKind::Disconnect), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

async fn an_unreachable_switch_target_keeps_the_player(version: ProtocolVersion) {
    assert_failed_switch_keeps_the_player(
        version,
        napping(ServerSpec::offline("b"), "B is napping"),
        None,
        FailedSwitch {
            message: "B is napping",
            cause: "unreachable",
            reason: Value::Null,
        },
    )
    .await;
}

version_matrix!(TEXT, an_unreachable_switch_target_keeps_the_player);

async fn a_refusing_switch_target_keeps_the_player(version: ProtocolVersion) {
    let backend_b = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("B is full"))
        .spawn()
        .await
        .unwrap();
    assert_failed_switch_keeps_the_player(
        version,
        ServerSpec::offline("b").backend(backend_b.addr()),
        Some(&backend_b),
        FailedSwitch {
            message: "B is full",
            cause: "login_refused",
            reason: text_value("B is full"),
        },
    )
    .await;
}

version_matrix!(TEXT, a_refusing_switch_target_keeps_the_player);

async fn a_config_kick_during_a_switch_keeps_the_player(version: ProtocolVersion) {
    let backend_b = FakeBackend::builder().hold_config().spawn().await.unwrap();
    assert_failed_switch_keeps_the_player(
        version,
        ServerSpec::offline("b").backend(backend_b.addr()),
        Some(&backend_b),
        FailedSwitch {
            message: "B turned you away",
            cause: "config_disconnect",
            reason: text_value("B turned you away"),
        },
    )
    .await;
}

version_matrix!(a_config_kick_during_a_switch_keeps_the_player; p764 = 764, p774 = 774);

async fn an_unreachable_initial_server_shows_its_message(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(napping(ServerSpec::offline("lobby"), "Lobby is napping"))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.text, "Lobby is napping", "{info:?}");

    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!("unreachable"), "{kick:?}");
    assert_eq!(kick.detail["reason"], Value::Null, "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(kick.detail["previous_server"], json!(null), "{kick:?}");
    assert_eq!(
        kick.detail["result"],
        json!({ "disconnect_player": null }),
        "{kick:?}"
    );
    let disconnect = disconnected(&recorder).await;
    assert_eq!(disconnect.detail["cause"], json!("error"));
    assert!(disconnect.seq > kick.seq);

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, an_unreachable_initial_server_shows_its_message);

async fn an_unreachable_initial_server_falls_back_to_its_limbo(version: ProtocolVersion) {
    let recorder = Recorder::new();
    let (catch, mut holds) = catcher();
    let proxy = TestProxy::builder()
        .server(napping(ServerSpec::offline("lobby"), "Lobby is napping").limbo_handlers([CATCH]))
        .plugin(catch)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    let (gate, context) = next_hold(&mut holds).await;
    assert!(
        matches!(context, LimboEntryContext::InitialConnection { .. }),
        "{context:?}"
    );

    gate.complete(HandlerResult::Accept);

    let (parked, context) = next_hold(&mut holds).await;
    match context {
        LimboEntryContext::KickedFromServer { server, reason } => {
            assert_eq!(server, ServerId::new("lobby"));
            assert_eq!(reason, Component::text("Lobby is napping"));
        }
        other => panic!("expected a kick context, got {other:?}"),
    }
    sync(&mut session, player.as_ref()).await;
    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!("unreachable"), "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(kick.detail["previous_server"], json!(null), "{kick:?}");
    assert_eq!(
        kick.detail["result"],
        json!({ "send_to_limbo": [CATCH] }),
        "{kick:?}"
    );

    parked.complete(HandlerResult::Deny(Component::text("Try later")));
    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, "Try later", "{info:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, an_unreachable_initial_server_falls_back_to_its_limbo);

async fn a_refused_initial_login_shows_the_backend_reason(version: ProtocolVersion) {
    let sessions = FakeSessionServer::spawn().await.unwrap();
    let refusal = json!({ "color": "gold", "text": "Whitelisted only" });
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Refuse(refusal.clone()))
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::client_only("lobby").backend(backend.addr()))
        .session_server(&sessions)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.text, "Whitelisted only", "{info:?}");
    assert_eq!(info.json, Some(refusal.clone()), "{info:?}");

    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!("login_refused"), "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(
        kick.detail["reason"],
        component_value(&Component::text("Whitelisted only").color(NamedColor::Gold))
    );
    let disconnect = disconnected(&recorder).await;
    assert_eq!(disconnect.detail["cause"], json!("backend_closed"));
    assert_eq!(disconnect.detail["reason"], json!("Whitelisted only"));

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, a_refused_initial_login_shows_the_backend_reason);

async fn a_refused_forwarded_login_reaches_the_client_as_sent(version: ProtocolVersion) {
    let refusal = json!({ "color": "gold", "text": "Whitelisted only" });
    let backend = FakeBackend::builder()
        .login(LoginBehavior::Refuse(refusal.clone()))
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let info = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Login, "{info:?}");
    assert_eq!(info.raw, refusal.to_string().into_bytes(), "{info:?}");
    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!("login_refused"), "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, a_refused_forwarded_login_reaches_the_client_as_sent);

async fn a_config_kick_during_the_initial_join_reaches_the_client(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let client = proxy.client(version);
    let login = tokio::spawn(async move { client.login(STEVE).await });
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.state(), ConnectionState::Config);

    conn.kick_json(r#"{"text":"Config says no","color":"red"}"#)
        .await
        .unwrap();

    let info = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap()
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(info.state, ConnectionState::Config, "{info:?}");
    assert_eq!(info.text, "Config says no", "{info:?}");
    let reason = component_value(&Component::text("Config says no").color(NamedColor::Red));
    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!("config_disconnect"), "{kick:?}");
    assert_eq!(kick.detail["reason"], reason, "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(
        kick.detail["result"],
        json!({ "disconnect_player": null }),
        "{kick:?}"
    );
    let disconnect = disconnected(&recorder).await;
    assert_eq!(disconnect.detail["cause"], json!("backend_closed"));
    assert_eq!(disconnect.detail["reason_json"], reason);

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_config_kick_during_the_initial_join_reaches_the_client; p764 = 764, p774 = 774);

async fn kick_redirects_stop_after_three_attempts(version: ProtocolVersion) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("B refuses"))
        .spawn()
        .await
        .unwrap();
    let backend_c = FakeBackend::builder()
        .login(LoginBehavior::refuse_text("C refuses"))
        .spawn()
        .await
        .unwrap();
    let recorder = Recorder::new();
    let bounce = on_kick("bounce", |e| {
        let next = if e.server == ServerId::new("b") {
            "c"
        } else {
            "b"
        };
        e.redirect_to(ServerId::new(next));
    });
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .server(network(ServerSpec::offline("c").backend(backend_c.addr())))
        .plugin(bounce)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();
    recorder
        .wait_for_kind(EventKind::ServerPostConnect, T)
        .await
        .unwrap();

    conn_a.kick_json(r#"{"text":"A kicks"}"#).await.unwrap();

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.state, ConnectionState::Play, "{info:?}");
    assert_eq!(info.text, "B refuses", "{info:?}");
    disconnected(&recorder).await;
    let kicks: Vec<Value> = recorder
        .of(EventKind::KickedFromServer)
        .iter()
        .map(|e| e.detail["server"].clone())
        .collect();
    assert_eq!(kicks, [json!("a"), json!("b"), json!("c"), json!("b")]);
    let redirects = recorder.filter(|e| {
        e.kind == EventKind::ServerPreConnect && e.detail["cause"] == json!("kick_redirect")
    });
    assert_eq!(redirects.len(), 3, "{redirects:?}");

    proxy.shutdown().await.unwrap();
}

version_matrix!(TEXT, kick_redirects_stop_after_three_attempts);

async fn assert_initial_redirect_joins(
    version: ProtocolVersion,
    lobby: ServerSpec,
    lobby_backend: Option<&FakeBackend>,
    cause: &'static str,
) {
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let redirect = on_kick("redirect", |e| {
        if e.server == ServerId::new("lobby") {
            e.redirect_to(ServerId::new("b"));
        }
    });
    let proxy = TestProxy::builder()
        .server(network(lobby))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(redirect)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let client = proxy.client_for("lobby", version).unwrap();
    let login = tokio::spawn(async move { client.login(STEVE).await });
    let _kicking = match lobby_backend {
        Some(backend) => {
            let mut conn = backend.next_connection(T).await.unwrap();
            conn.kick_json(r#"{"text":"Lobby says no"}"#).await.unwrap();
            Some(conn)
        }
        None => None,
    };

    let mut session = tokio::time::timeout(T, login)
        .await
        .expect("the login must end")
        .unwrap()
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    sync(&mut session, player.as_ref()).await;
    assert_eq!(player.current_server(), Some(ServerId::new("b")));

    let kick = kicked(&recorder, "lobby").await;
    assert_eq!(kick.detail["cause"], json!(cause), "{kick:?}");
    assert_eq!(kick.detail["during_connect"], json!(true), "{kick:?}");
    assert_eq!(kick.detail["result"], json!({ "redirect_to": "b" }));
    let to_b = recorder.filter(|e| e.detail["server"] == json!("b"));
    let kinds: Vec<EventKind> = to_b.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::ServerPostConnect
        ]
    );
    assert_eq!(to_b[0].detail["cause"], json!("kick_redirect"));
    assert_eq!(to_b[0].detail["previous_server"], json!(null));

    session.chat("hello b").await.unwrap();
    assert_eq!(
        conn_b.expect::<SChatMessage>(T).await.unwrap().message,
        "hello b"
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

async fn an_unreachable_initial_server_can_redirect(version: ProtocolVersion) {
    assert_initial_redirect_joins(
        version,
        ServerSpec::offline("lobby").unreachable(),
        None,
        "unreachable",
    )
    .await;
}

version_matrix!(TEXT, an_unreachable_initial_server_can_redirect);

async fn a_config_kick_during_the_initial_join_can_redirect(version: ProtocolVersion) {
    let lobby = FakeBackend::builder().hold_config().spawn().await.unwrap();
    assert_initial_redirect_joins(
        version,
        ServerSpec::offline("lobby").backend(lobby.addr()),
        Some(&lobby),
        "config_disconnect",
    )
    .await;
}

version_matrix!(a_config_kick_during_the_initial_join_can_redirect; p764 = 764, p774 = 774);

async fn a_redirect_after_a_kick_mid_reconfiguration_absorbs_the_late_ack(
    version: ProtocolVersion,
) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (deciding_tx, mut deciding) = mpsc::unbounded_channel();
    let (release, released) = watch::channel(false);
    let redirect = ScriptedPlugin::new("redirect").on_async::<KickedFromServerEvent>(
        EventPriority::NORMAL,
        move |event| {
            let deciding_tx = deciding_tx.clone();
            let mut released = released.clone();
            Box::pin(async move {
                let _ = deciding_tx.send(());
                let _ = released.wait_for(|go| *go).await;
                event.redirect_to(ServerId::new("b"));
            })
        },
    );
    let proxy = TestProxy::builder()
        .server(network(ServerSpec::offline("a").backend(backend_a.addr())))
        .server(network(ServerSpec::offline("b").backend(backend_b.addr())))
        .plugin(redirect)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    let mut session = proxy
        .client_for("a", version)
        .unwrap()
        .hold_configuration_ack()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn_a = backend_a.next_connection(T).await.unwrap();

    conn_a.start_configuration().await.unwrap();
    conn_a.kick_json(r#"{"text":"Restarting"}"#).await.unwrap();
    tokio::time::timeout(T, deciding.recv())
        .await
        .unwrap()
        .unwrap();
    let early = wire::encode(
        &SChatMessage {
            message: "sent before the acknowledgement".to_string(),
            ..SChatMessage::default()
        },
        version,
    )
    .unwrap();
    session.send_frame(&early).await.unwrap();
    session
        .send_packet(&SAcknowledgeConfiguration)
        .await
        .unwrap();
    let acked = plugin_message::to_backend(
        &PluginMessage::new(ACKED, b"acked".to_vec()),
        ConnectionState::Config,
        version,
    )
    .unwrap();
    session.send_frame(&acked).await.unwrap();
    release.send_replace(true);

    let mut conn_b = backend_b.next_connection(T).await.unwrap();
    plugin_message::backend_message(&mut conn_b, ACKED, T)
        .await
        .unwrap();
    conn_b.finish_config(T).await.unwrap();
    session.expect_join(T).await.unwrap();

    let ack_id = wire::encode(&SAcknowledgeConfiguration, version)
        .unwrap()
        .id;
    let late = |frame: &PacketFrame| {
        frame.id == ack_id || (frame.id == early.id && frame.payload == early.payload)
    };
    let stray: Vec<_> = conn_b.received().into_iter().filter(|f| late(f)).collect();
    assert!(
        stray.is_empty(),
        "play packets the client sent before acknowledging the kicking server's configuration must not reach the redirect target: {stray:?}"
    );
    let leaked: Vec<_> = conn_a.received().into_iter().filter(|f| late(f)).collect();
    assert!(
        leaked.is_empty(),
        "nothing is forwarded to the server that kicked the player: {leaked:?}"
    );

    let kick = kicked(&recorder, "a").await;
    assert_eq!(kick.detail["cause"], json!("config_disconnect"));
    assert_eq!(kick.detail["during_connect"], json!(false));
    session.chat("landed").await.unwrap();
    assert_eq!(
        conn_b.expect::<SChatMessage>(T).await.unwrap().message,
        "landed"
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_redirect_after_a_kick_mid_reconfiguration_absorbs_the_late_ack;
    p764 = 764, p766 = 766, p774 = 774);
