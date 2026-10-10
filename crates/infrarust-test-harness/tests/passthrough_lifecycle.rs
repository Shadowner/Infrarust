#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use infrarust_api::error::PlayerError;
use infrarust_api::event::EventPriority;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    KickedFromServerEvent, PlayerChooseInitialServerEvent, ServerPreConnectEvent,
    ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{GameProfileRequestEvent, LoginEvent, PreLoginEvent};
use infrarust_api::types::{Component, NamedColor, ProfileProperty, ServerId};
use infrarust_config::ProxyMode;
use infrarust_test_harness::{
    ConnectionState, DEFAULT_TIMEOUT, EventKind, FakeBackend, ProtocolVersion, Recorded, Recorder,
    ScriptedPlugin, ServerSpec, TestProxy,
};
use serde_json::{Value, json};
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const STEVE: &str = "Steve";
const LIMBO_REFUSED: &str = "Limbo is not available on this server";

const ORDER: [EventKind; 9] = [
    EventKind::PreLogin,
    EventKind::GameProfileRequest,
    EventKind::PermissionsSetup,
    EventKind::Login,
    EventKind::PostLogin,
    EventKind::PlayerChooseInitialServer,
    EventKind::ServerPreConnect,
    EventKind::ServerConnected,
    EventKind::Disconnect,
];

macro_rules! family {
    ($body:ident) => {
        mod $body {
            use super::*;

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough_p47() {
                super::$body(ProxyMode::Passthrough, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough_p764() {
                super::$body(ProxyMode::Passthrough, ProtocolVersion(764)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn passthrough_current() {
                super::$body(
                    ProxyMode::Passthrough,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn zero_copy_p47() {
                super::$body(ProxyMode::ZeroCopy, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn zero_copy_p764() {
                super::$body(ProxyMode::ZeroCopy, ProtocolVersion(764)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn zero_copy_current() {
                super::$body(
                    ProxyMode::ZeroCopy,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn server_only_p47() {
                super::$body(ProxyMode::ServerOnly, ProtocolVersion(47)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn server_only_p764() {
                super::$body(ProxyMode::ServerOnly, ProtocolVersion(764)).await;
            }

            #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
            async fn server_only_current() {
                super::$body(
                    ProxyMode::ServerOnly,
                    ProtocolVersion(infrarust_test_harness::versions::CURRENT),
                )
                .await;
            }
        }
    };
}

fn kinds(recorder: &Recorder, username: &str) -> Vec<EventKind> {
    recorder
        .for_username(username)
        .iter()
        .map(|e| e.kind)
        .filter(|kind| ORDER.contains(kind) || *kind == EventKind::KickedFromServer)
        .collect()
}

async fn disconnected(recorder: &Recorder, username: &str) -> Recorded {
    recorder
        .wait_for(
            |e| e.kind == EventKind::Disconnect && e.is_named(username),
            T,
        )
        .await
        .unwrap()
}

async fn assert_paired(proxy: &TestProxy, recorder: &Recorder, username: &str) -> Recorded {
    let disconnect = disconnected(recorder, username).await;
    proxy.wait_for_connection_count(0, T).await.unwrap();
    let post_logins = recorder.of(EventKind::PostLogin);
    assert_eq!(post_logins.len(), 1, "{post_logins:?}");
    assert_eq!(recorder.count(EventKind::Disconnect), 1);
    assert!(post_logins[0].seq < disconnect.seq);
    assert_eq!(post_logins[0].player, disconnect.player);
    disconnect
}

fn assert_never_admitted(proxy: &TestProxy, recorder: &Recorder, backend: &FakeBackend) {
    assert_eq!(recorder.count(EventKind::PostLogin), 0);
    assert_eq!(recorder.count(EventKind::Disconnect), 0);
    assert_eq!(proxy.connection_count(), 0);
    assert_eq!(backend.accepted_connections(), 0);
}

fn styled(text: &str) -> Component {
    Component::text(text).color(NamedColor::Red).bold()
}

async fn events_fire_once_in_order(mode: ProxyMode, version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), STEVE);
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    assert!(!player.is_active());
    assert!(matches!(
        player.send_message(Component::text("hi")),
        Err(PlayerError::NotActive)
    ));
    assert_eq!(player.current_server(), Some(ServerId::new("lobby")));

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    let disconnect = assert_paired(&proxy, &recorder, STEVE).await;

    assert_eq!(kinds(&recorder, STEVE), ORDER, "{mode:?} {}", version.0);
    let player_id = disconnect.player.expect("Disconnect carries the player");
    for event in recorder.for_username(STEVE) {
        if event.player.is_some() {
            assert_eq!(event.player, Some(player_id), "{}", event.kind);
        }
    }
    let pre_login = &recorder.of(EventKind::PreLogin)[0];
    assert_eq!(pre_login.server_domain(), Some("lobby.test"));
    assert_eq!(pre_login.protocol_version(), version.0);
    let request = &recorder.of(EventKind::GameProfileRequest)[0];
    assert!(!request.online_mode());
    assert_eq!(request.virtual_host(), Some("lobby.test"));
    assert!(!recorder.of(EventKind::PermissionsSetup)[0].online_mode());
    assert!(!recorder.of(EventKind::Login)[0].online_mode());
    let pre_connect = &recorder.of(EventKind::ServerPreConnect)[0];
    assert_eq!(pre_connect.server(), Some("lobby"));
    assert_eq!(pre_connect.cause(), "initial");
    assert_forwarding_ended(&disconnect);
    assert_eq!(disconnect.last_server(), Some("lobby"));

    proxy.shutdown().await.unwrap();
}

family!(events_fire_once_in_order);

async fn a_pre_login_denial_ends_in_login(mode: ProxyMode, version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let gate = ScriptedPlugin::new("gate").on::<PreLoginEvent>(EventPriority::NORMAL, |event| {
        if event.profile.username == "Mallory" {
            event.deny(styled("Mallory is not welcome"));
        }
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "Mallory is not welcome", "{denied:?}");
    assert_eq!(
        denied.json,
        Some(json!({ "text": "Mallory is not welcome", "color": "red", "bold": true }))
    );
    assert_eq!(kinds(&recorder, "Mallory"), [EventKind::PreLogin]);
    assert_never_admitted(&proxy, &recorder, &backend);

    proxy.shutdown().await.unwrap();
}

family!(a_pre_login_denial_ends_in_login);

async fn a_login_denial_ends_in_login(mode: ProxyMode, version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let gate = ScriptedPlugin::new("gate").on::<LoginEvent>(EventPriority::NORMAL, |event| {
        if event.profile().username == "Mallory" {
            event.deny(Component::text("No entry for Mallory"));
        }
    });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(gate)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login("Mallory")
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "No entry for Mallory", "{denied:?}");
    assert_eq!(
        kinds(&recorder, "Mallory"),
        [
            EventKind::PreLogin,
            EventKind::GameProfileRequest,
            EventKind::PermissionsSetup,
            EventKind::Login
        ]
    );
    assert_eq!(
        *recorder.of(EventKind::Login)[0].result(),
        json!({ "denied": "No entry for Mallory" })
    );
    assert_never_admitted(&proxy, &recorder, &backend);

    proxy.shutdown().await.unwrap();
}

family!(a_login_denial_ends_in_login);

async fn assert_joined_other(
    proxy: &TestProxy,
    recorder: &Recorder,
    lobby: &FakeBackend,
    other: &FakeBackend,
    version: ProtocolVersion,
) {
    let session = proxy
        .client_for("lobby", version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = other.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), STEVE);
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    assert_eq!(player.current_server(), Some(ServerId::new("other")));

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    let disconnect = assert_paired(proxy, recorder, STEVE).await;
    assert_eq!(disconnect.last_server(), Some("other"));
    let connected = recorder.of(EventKind::ServerConnected);
    assert_eq!(connected.len(), 1, "{connected:?}");
    assert_eq!(connected[0].server(), Some("other"));
    assert_eq!(lobby.accepted_connections(), 0);
}

async fn an_initial_server_redirect_reaches_the_target(mode: ProxyMode, version: ProtocolVersion) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let other = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let chooser = ScriptedPlugin::new("chooser")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, |event| {
            event.redirect_to(ServerId::new("other"))
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(lobby.addr()))
        .server(ServerSpec::new("other", mode).backend(other.addr()))
        .plugin(chooser)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_joined_other(&proxy, &recorder, &lobby, &other, version).await;
    let pre_connects = recorder.of(EventKind::ServerPreConnect);
    assert_eq!(pre_connects.len(), 1, "{pre_connects:?}");
    assert_eq!(pre_connects[0].server(), Some("other"));
    assert_eq!(pre_connects[0].cause(), "initial");

    proxy.shutdown().await.unwrap();
}

family!(an_initial_server_redirect_reaches_the_target);

async fn a_pre_connect_redirect_reaches_the_target(mode: ProxyMode, version: ProtocolVersion) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let other = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let router =
        ScriptedPlugin::new("router").on::<ServerPreConnectEvent>(EventPriority::NORMAL, |event| {
            if event.server == ServerId::new("lobby") {
                event.redirect_to(ServerId::new("other"));
            }
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(lobby.addr()))
        .server(ServerSpec::new("other", mode).backend(other.addr()))
        .plugin(router)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_joined_other(&proxy, &recorder, &lobby, &other, version).await;

    proxy.shutdown().await.unwrap();
}

family!(a_pre_connect_redirect_reaches_the_target);

async fn assert_limbo_refused(
    proxy: &TestProxy,
    recorder: &Recorder,
    backend: &FakeBackend,
    version: ProtocolVersion,
) {
    let denied = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();
    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, LIMBO_REFUSED, "{denied:?}");
    let disconnect = assert_paired(proxy, recorder, STEVE).await;
    assert_eq!(disconnect.cause(), "kicked");
    assert_eq!(disconnect.reason(), Some(LIMBO_REFUSED));
    assert_eq!(disconnect.last_server(), None);
    assert_eq!(recorder.count(EventKind::ServerConnected), 0);
    assert_eq!(backend.accepted_connections(), 0);
}

async fn an_initial_limbo_request_fails_closed(mode: ProxyMode, version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let chooser = ScriptedPlugin::new("chooser")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, |event| {
            event.send_to_limbo(vec!["queue".into()])
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(chooser)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_limbo_refused(&proxy, &recorder, &backend, version).await;
    assert_eq!(recorder.count(EventKind::ServerPreConnect), 0);

    proxy.shutdown().await.unwrap();
}

family!(an_initial_limbo_request_fails_closed);

async fn a_pre_connect_limbo_request_fails_closed(mode: ProxyMode, version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let parker =
        ScriptedPlugin::new("parker").on::<ServerPreConnectEvent>(EventPriority::NORMAL, |event| {
            event.set_result(ServerPreConnectResult::SendToLimbo {
                limbo_handlers: vec!["queue".into()],
            });
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(backend.addr()))
        .plugin(parker)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    assert_limbo_refused(&proxy, &recorder, &backend, version).await;

    proxy.shutdown().await.unwrap();
}

family!(a_pre_connect_limbo_request_fails_closed);

fn napping(spec: ServerSpec) -> ServerSpec {
    spec.unreachable().patch(|table| {
        table.insert(
            "disconnect_message".into(),
            toml::Value::String("Lobby is napping".into()),
        );
    })
}

async fn an_unreachable_backend_disconnects_by_default(mode: ProxyMode, version: ProtocolVersion) {
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(napping(ServerSpec::new("lobby", mode)))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(denied.text, "Lobby is napping", "{denied:?}");
    let disconnect = assert_paired(&proxy, &recorder, STEVE).await;
    assert_eq!(disconnect.cause(), "error");
    let kicks = recorder.of(EventKind::KickedFromServer);
    assert_eq!(kicks.len(), 1, "{kicks:?}");
    assert_eq!(kicks[0].server(), Some("lobby"));
    assert_eq!(kicks[0].cause(), "unreachable");
    assert!(kicks[0].during_connect());
    assert_eq!(kicks[0].previous_server(), None);
    assert_eq!(kicks[0].kick_reason(), None);
    assert_eq!(*kicks[0].result(), json!({ "disconnect_player": null }));
    assert_eq!(recorder.count(EventKind::ServerConnected), 0);

    proxy.shutdown().await.unwrap();
}

family!(an_unreachable_backend_disconnects_by_default);

async fn an_unreachable_backend_can_redirect(mode: ProxyMode, version: ProtocolVersion) {
    let other = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let rescuer = ScriptedPlugin::new("rescuer").on::<KickedFromServerEvent>(
        EventPriority::NORMAL,
        |event| {
            if event.server == ServerId::new("lobby") {
                event.redirect_to(ServerId::new("other"));
            }
        },
    );
    let proxy = TestProxy::builder()
        .server(napping(ServerSpec::new("lobby", mode)))
        .server(ServerSpec::new("other", mode).backend(other.addr()))
        .plugin(rescuer)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client_for("lobby", version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = other.next_connection(T).await.unwrap();
    assert_eq!(conn.username(), STEVE);

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    let disconnect = assert_paired(&proxy, &recorder, STEVE).await;
    assert_eq!(disconnect.last_server(), Some("other"));
    assert_forwarding_ended(&disconnect);

    assert_eq!(
        kinds(&recorder, STEVE),
        [
            EventKind::PreLogin,
            EventKind::GameProfileRequest,
            EventKind::PermissionsSetup,
            EventKind::Login,
            EventKind::PostLogin,
            EventKind::PlayerChooseInitialServer,
            EventKind::ServerPreConnect,
            EventKind::KickedFromServer,
            EventKind::ServerPreConnect,
            EventKind::ServerConnected,
            EventKind::Disconnect,
        ]
    );
    let kick = &recorder.of(EventKind::KickedFromServer)[0];
    assert_eq!(kick.cause(), "unreachable");
    assert!(kick.during_connect());
    assert_eq!(*kick.result(), json!({ "redirect_to": "other" }));
    let pre_connects = recorder.of(EventKind::ServerPreConnect);
    assert_eq!(pre_connects[1].server(), Some("other"));
    assert_eq!(pre_connects[1].cause(), "kick_redirect");
    assert_eq!(pre_connects[1].previous_server(), None);
    let connected = &recorder.of(EventKind::ServerConnected)[0];
    assert_eq!(connected.server(), Some("other"));

    proxy.shutdown().await.unwrap();
}

family!(an_unreachable_backend_can_redirect);

const REWRITTEN: Uuid = Uuid::from_u128(0x00ff_00ff_00ff_00ff_00ff_00ff_00ff_00ff);

async fn a_rewritten_profile_reaches_bungeecord_forwarding(
    mode: ProxyMode,
    version: ProtocolVersion,
) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let skins = ScriptedPlugin::new("skins").on::<GameProfileRequestEvent>(
        EventPriority::NORMAL,
        |event| {
            event.profile.uuid = REWRITTEN;
            event.profile.properties = vec![ProfileProperty {
                name: "textures".into(),
                value: "skin".into(),
                signature: None,
            }];
        },
    );
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::new("lobby", mode)
                .backend(backend.addr())
                .patch(|table| {
                    table.insert(
                        "forwarding_mode".into(),
                        toml::Value::String("legacy".into()),
                    );
                }),
        )
        .plugin(skins)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let session = proxy
        .client(version)
        .login(STEVE)
        .await
        .unwrap()
        .joined()
        .unwrap();
    let mut conn = backend.next_connection(T).await.unwrap();

    assert_eq!(conn.username(), STEVE);
    let forwarded: Vec<&str> = conn.handshake().server_address.split('\0').collect();
    assert_eq!(forwarded.len(), 4, "{forwarded:?}");
    assert_eq!(forwarded[0], "lobby.test");
    assert_eq!(forwarded[1], "127.0.0.1");
    assert_eq!(forwarded[2], REWRITTEN.simple().to_string());
    let properties: Value = serde_json::from_str(forwarded[3]).unwrap();
    assert_eq!(properties, json!([{ "name": "textures", "value": "skin" }]));
    let player = proxy.wait_for_player(STEVE, T).await.unwrap();
    assert_eq!(player.profile().uuid, REWRITTEN);
    let post_login = &recorder.of(EventKind::PostLogin)[0];
    assert_eq!(post_login.profile_uuid(), json!(REWRITTEN.to_string()));
    drop(player);

    session.quit().await;
    conn.closed(T).await.unwrap();
    conn.close().await;
    assert_paired(&proxy, &recorder, STEVE).await;

    proxy.shutdown().await.unwrap();
}

family!(a_rewritten_profile_reaches_bungeecord_forwarding);

async fn a_redirect_to_a_proxy_login_server_fails_closed(
    mode: ProxyMode,
    version: ProtocolVersion,
) {
    let lobby = FakeBackend::builder().spawn().await.unwrap();
    let secure = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let chooser = ScriptedPlugin::new("chooser")
        .on::<PlayerChooseInitialServerEvent>(EventPriority::NORMAL, |event| {
            event.redirect_to(ServerId::new("secure"))
        });
    let proxy = TestProxy::builder()
        .server(ServerSpec::new("lobby", mode).backend(lobby.addr()))
        .server(ServerSpec::client_only("secure").backend(secure.addr()))
        .plugin(chooser)
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let denied = proxy
        .client_for("lobby", version)
        .unwrap()
        .login(STEVE)
        .await
        .unwrap()
        .disconnected()
        .unwrap();

    assert_eq!(denied.state, ConnectionState::Login);
    assert_eq!(
        denied.text, "This server cannot be joined from here",
        "{denied:?}"
    );
    let disconnect = assert_paired(&proxy, &recorder, STEVE).await;
    assert_eq!(disconnect.cause(), "kicked");
    assert_eq!(recorder.count(EventKind::ServerConnected), 0);
    assert_eq!(lobby.accepted_connections(), 0);
    assert_eq!(secure.accepted_connections(), 0);

    proxy.shutdown().await.unwrap();
}

family!(a_redirect_to_a_proxy_login_server_fails_closed);

fn assert_forwarding_ended(disconnect: &Recorded) {
    assert_eq!(disconnect.cause(), "client_quit", "{disconnect:?}");
    assert_eq!(disconnect.reason(), None, "{disconnect:?}");
}
