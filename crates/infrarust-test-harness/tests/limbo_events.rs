#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::event::{EventPriority, ResultedEvent};
use infrarust_api::events::connection::{KickedFromServerEvent, KickedFromServerResult};
use infrarust_api::limbo::handler::HandlerResult;
use infrarust_api::types::{Component, ServerId};
use infrarust_test_harness::{
    DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorded, Recorder, ScriptedPlugin,
    ServerSpec, TestProxy, holding_gate, next_hold, version_matrix,
};
use serde_json::{Value, json};

const T: Duration = DEFAULT_TIMEOUT;
const STEVE: &str = "Steve";
const GATE: &str = "gate";

fn steve(recorder: &Recorder) -> Vec<Recorded> {
    recorder.filter(|e| e.username.as_deref() == Some(STEVE) && e.player.is_some())
}

fn kinds(events: &[Recorded]) -> Vec<EventKind> {
    events.iter().map(|e| e.kind).collect()
}

fn one(events: &[Recorded], kind: EventKind) -> Value {
    let found: Vec<&Recorded> = events.iter().filter(|e| e.kind == kind).collect();
    assert_eq!(found.len(), 1, "{kind}: {events:#?}");
    found[0].detail.clone()
}

async fn an_initial_gate_enters_and_leaves_limbo_before_the_server(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers([GATE]),
        )
        .plugin(gate)
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
    let handle = next_hold(&mut holds, T).await.unwrap().handle;
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    assert_eq!(recorder.count(EventKind::LimboEnter), 1);
    assert_eq!(recorder.count(EventKind::LimboExit), 0);

    handle.complete(HandlerResult::Accept);
    let _conn = backend.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    session.sync_with(player.as_ref(), T).await.unwrap();

    let events = steve(&recorder);
    let from_post_login: Vec<EventKind> = kinds(&events)
        .into_iter()
        .skip_while(|kind| *kind != EventKind::PostLogin)
        .collect();
    assert_eq!(
        from_post_login,
        [
            EventKind::PostLogin,
            EventKind::PlayerChooseInitialServer,
            EventKind::ServerPreConnect,
            EventKind::LimboEnter,
            EventKind::LimboExit,
            EventKind::ServerConnected,
            EventKind::ServerPostConnect,
        ]
    );
    assert_eq!(
        one(&events, EventKind::LimboEnter),
        json!({
            "handlers": [GATE],
            "context": format!(
                "{:?}",
                infrarust_api::limbo::context::LimboEntryContext::InitialConnection {
                    target_server: ServerId::new("lobby"),
                }
            ),
            "current_server": null,
        })
    );
    assert_eq!(
        one(&events, EventKind::LimboExit),
        json!({
            "reason": "released",
            "kick_reason": null,
            "handlers": null,
            "next_server": "lobby",
        })
    );

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(an_initial_gate_enters_and_leaves_limbo_before_the_server; p47 = 47, p764 = 764, p774 = 774);

async fn a_limbo_redirect_names_the_next_server(version: ProtocolVersion) {
    let hub = FakeBackend::builder().spawn().await.unwrap();
    let game = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("hub")
                .backend(hub.addr())
                .network("main")
                .limbo_handlers([GATE]),
        )
        .server(
            ServerSpec::offline("game")
                .backend(game.addr())
                .network("main"),
        )
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let mut session = proxy
        .client_for("hub", version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let handle = next_hold(&mut holds, T).await.unwrap().handle;
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();

    handle.complete(HandlerResult::Redirect(ServerId::new("game")));
    let _conn = game.next_connection(T).await.unwrap();
    session.expect_join(T).await.unwrap();
    session.sync_with(player.as_ref(), T).await.unwrap();

    let events = steve(&recorder);
    let exit = events
        .iter()
        .position(|e| e.kind == EventKind::LimboExit)
        .unwrap();
    assert_eq!(events[exit].reason(), Some("redirected"));
    assert_eq!(events[exit].next_server(), Some("game"));
    let pre_connect = &events[exit + 1];
    assert_eq!(pre_connect.kind, EventKind::ServerPreConnect, "{events:#?}");
    assert_eq!(pre_connect.server(), Some("game"));
    assert_eq!(pre_connect.cause(), "limbo_exit");
    assert_eq!(hub.accepted_connections(), 0);

    session.quit().await;
    proxy.shutdown().await.unwrap();
}

version_matrix!(a_limbo_redirect_names_the_next_server; p47 = 47, p774 = 774);

async fn a_kick_to_limbo_then_a_disconnect_exits_before_the_disconnect_event(
    version: ProtocolVersion,
) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let to_limbo = ScriptedPlugin::new("to_limbo").on::<KickedFromServerEvent>(
        EventPriority::NORMAL,
        |event| {
            event.set_result(KickedFromServerResult::SendToLimbo {
                limbo_handlers: vec![GATE.to_string()],
            });
        },
    );
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(gate)
        .plugin(to_limbo)
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
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    session.sync_with(player.as_ref(), T).await.unwrap();

    conn.kick_json(r#"{"text":"Crashed"}"#).await.unwrap();
    let _hold = next_hold(&mut holds, T).await.unwrap();
    player.disconnect(Component::text("Bye from limbo")).await;

    let info = session.expect_disconnect(T).await.unwrap();
    assert_eq!(info.text, "Bye from limbo");
    recorder
        .wait_for(|e| e.kind == EventKind::Disconnect, T)
        .await
        .unwrap();

    let events = steve(&recorder);
    let tail: Vec<EventKind> = kinds(&events)
        .into_iter()
        .skip_while(|kind| *kind != EventKind::KickedFromServer)
        .collect();
    assert_eq!(
        tail,
        [
            EventKind::KickedFromServer,
            EventKind::LimboEnter,
            EventKind::LimboExit,
            EventKind::Disconnect,
        ]
    );
    let enter = one(&events, EventKind::LimboEnter);
    assert_eq!(enter["handlers"], json!([GATE]));
    assert!(
        enter["context"]
            .as_str()
            .unwrap()
            .starts_with("KickedFromServer"),
        "{enter}"
    );
    let exit = one(&events, EventKind::LimboExit);
    assert_eq!(exit["reason"], json!("kicked"));
    assert_eq!(exit["kick_reason"], json!("Bye from limbo"));
    assert_eq!(exit["next_server"], Value::Null);
    assert_eq!(
        one(&events, EventKind::Disconnect)["cause"],
        json!("kicked")
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(a_kick_to_limbo_then_a_disconnect_exits_before_the_disconnect_event; p47 = 47, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shutdown_in_limbo_exits_before_the_disconnect_event() {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let (gate, mut holds) = holding_gate("gatekeeper", GATE);
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("lobby")
                .backend(backend.addr())
                .limbo_handlers([GATE]),
        )
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let _session = proxy
        .client(ProtocolVersion(774))
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let _hold = next_hold(&mut holds, T).await.unwrap();

    proxy.shutdown().await.unwrap();

    let events = steve(&recorder);
    let tail: Vec<EventKind> = kinds(&events)
        .into_iter()
        .skip_while(|kind| *kind != EventKind::LimboEnter)
        .collect();
    assert_eq!(
        tail,
        [
            EventKind::LimboEnter,
            EventKind::LimboExit,
            EventKind::Disconnect
        ]
    );
    assert_eq!(
        one(&events, EventKind::LimboExit)["reason"],
        json!("shutdown")
    );
    assert_eq!(
        one(&events, EventKind::Disconnect)["cause"],
        json!("shutdown")
    );
}
