#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::error::PlayerError;
use infrarust_api::player::{
    BossBar, BossBarColor, BossBarFlags, BossBarOverlay, ConnectionResult, Player,
    ResourcePackRequest,
};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::config::CConfigPluginMessage;
use infrarust_protocol::packets::play::boss_bar::{BossBarAction, CBossBar};
use infrarust_protocol::packets::play::chat::{
    CChatMessageLegacy, CSystemChatMessage, SChatMessage,
};
use infrarust_protocol::packets::play::tab_list::CTabListHeaderFooter;
use infrarust_protocol::packets::play::title::{CClearTitles, CTitleLegacy};
use infrarust_protocol::packets::resource_pack::{
    CConfigResourcePack, CConfigResourcePackPush, CResourcePack, CResourcePackPop,
    CResourcePackPush, ResourcePackResult, SConfigResourcePackResponse, SResourcePackResponse,
};
use infrarust_test_harness::plugin_message;
use infrarust_test_harness::text::component_text;
use infrarust_test_harness::{
    BackendConn, ClientSession, DEFAULT_TIMEOUT, EventKind, FakeBackend, LoginOutcome, PacketFrame,
    ProtocolVersion, Recorded, Recorder, ServerSpec, TestProxy, version_matrix, wire,
};
use uuid::Uuid;

const T: Duration = DEFAULT_TIMEOUT;
const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
const PACK_URL: &str = "https://packs.example.com/pack.zip";

fn system_text(frame: &PacketFrame, version: ProtocolVersion) -> Option<String> {
    if wire::is::<CSystemChatMessage>(frame, version) {
        let message = wire::decode::<CSystemChatMessage>(frame, version).ok()?;
        return Some(component_text(&message.content, version));
    }
    if wire::is::<CChatMessageLegacy>(frame, version) {
        let message = wire::decode::<CChatMessageLegacy>(frame, version).ok()?;
        return Some(component_text(message.content.as_bytes(), version));
    }
    None
}

async fn frames_until_marker(
    session: &mut ClientSession,
    player: &dyn Player,
    marker: &str,
) -> Vec<PacketFrame> {
    player.send_message(Component::text(marker)).unwrap();
    let version = session.version();
    let mut seen = Vec::new();
    loop {
        let frame = session.recv_frame(T).await.unwrap();
        if system_text(&frame, version).as_deref() == Some(marker) {
            return seen;
        }
        seen.push(frame);
    }
}

fn boss_bars(frames: &[PacketFrame], version: ProtocolVersion) -> Vec<CBossBar> {
    frames
        .iter()
        .filter(|frame| wire::is::<CBossBar>(frame, version))
        .map(|frame| wire::decode::<CBossBar>(frame, version).unwrap())
        .collect()
}

async fn two_servers() -> (FakeBackend, FakeBackend, TestProxy, Recorder) {
    let backend_a = FakeBackend::builder().spawn().await.unwrap();
    let backend_b = FakeBackend::builder().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(
            ServerSpec::offline("a")
                .backend(backend_a.addr())
                .network("main"),
        )
        .server(
            ServerSpec::offline("b")
                .backend(backend_b.addr())
                .network("main"),
        )
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();
    (backend_a, backend_b, proxy, recorder)
}

async fn join(proxy: &TestProxy, version: ProtocolVersion) -> (ClientSession, Arc<dyn Player>) {
    let session = proxy
        .client(version)
        .answer_resource_packs([
            ResourcePackResult::Accepted,
            ResourcePackResult::SuccessfullyLoaded,
        ])
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    (session, player)
}

fn header_footer_text(packet: &CTabListHeaderFooter, version: ProtocolVersion) -> (String, String) {
    (
        component_text(&packet.header, version),
        component_text(&packet.footer, version),
    )
}

async fn header_footer_survives_a_switch(version: ProtocolVersion) {
    let (_a, _b, proxy, _recorder) = two_servers().await;
    let (mut session, player) = join(&proxy, version).await;

    player
        .set_player_list_header_footer(Component::text("Top"), Component::text("Bottom"))
        .unwrap();
    let shown = session.expect::<CTabListHeaderFooter>(T).await.unwrap();
    assert_eq!(
        header_footer_text(&shown, version),
        ("Top".to_string(), "Bottom".to_string())
    );

    let result = player.connect(ServerId::new("b")).await.unwrap();
    assert_eq!(result, ConnectionResult::Success);
    session.expect_join(T).await.unwrap();
    let restored = session.expect::<CTabListHeaderFooter>(T).await.unwrap();
    assert_eq!(
        header_footer_text(&restored, version),
        ("Top".to_string(), "Bottom".to_string())
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(header_footer_survives_a_switch;
    p47 = 47, p340 = 340, p764 = 764, p765 = 765, p766 = 766, p774 = 774);

async fn clear_title_follows_the_protocol(version: ProtocolVersion) {
    let (_a, _b, proxy, _recorder) = two_servers().await;
    let (mut session, player) = join(&proxy, version).await;

    for reset in [false, true] {
        player.clear_title(reset).unwrap();
        if version.no_less_than(ProtocolVersion::V1_17) {
            let packet = session.expect::<CClearTitles>(T).await.unwrap();
            assert_eq!(packet.reset, reset);
        } else {
            let packet = session.expect::<CTitleLegacy>(T).await.unwrap();
            let expected = if reset {
                CTitleLegacy::Reset
            } else {
                CTitleLegacy::Hide
            };
            assert_eq!(packet, expected);
        }
    }

    proxy.shutdown().await.unwrap();
}

version_matrix!(clear_title_follows_the_protocol;
    p47 = 47, p340 = 340, p764 = 764, p765 = 765, p766 = 766, p774 = 774);

async fn boss_bar_is_added_updated_and_removed(version: ProtocolVersion) {
    let (_a, _b, proxy, _recorder) = two_servers().await;
    let (mut session, player) = join(&proxy, version).await;

    let bar = BossBar::new(Component::text("Dragon"))
        .progress(0.5)
        .color(BossBarColor::Red)
        .overlay(BossBarOverlay::Notched10)
        .flags(BossBarFlags::NONE.with(BossBarFlags::DARKEN_SCREEN));
    let handle = player.show_boss_bar(bar).unwrap();

    let added = session.expect::<CBossBar>(T).await.unwrap();
    assert_eq!(added.id, handle.id());
    match added.action {
        BossBarAction::Add {
            title,
            health,
            color,
            division,
            flags,
        } => {
            assert_eq!(component_text(&title, version), "Dragon");
            assert!((health - 0.5).abs() < f32::EPSILON);
            assert_eq!((color, division, flags), (2, 2, 1));
        }
        other => panic!("expected an add, got {other:?}"),
    }

    handle.set_progress(1.5).unwrap();
    handle.set_title(Component::text("Wither")).unwrap();
    handle
        .set_style(BossBarColor::Blue, BossBarOverlay::Notched6)
        .unwrap();
    handle
        .set_flags(BossBarFlags::NONE.with(BossBarFlags::CREATE_WORLD_FOG))
        .unwrap();
    handle.hide().unwrap();

    let mut updates = Vec::new();
    for _ in 0..5 {
        let packet = session.expect::<CBossBar>(T).await.unwrap();
        assert_eq!(packet.id, handle.id());
        updates.push(packet.action);
    }
    assert_eq!(updates[0], BossBarAction::UpdateHealth(1.0));
    match &updates[1] {
        BossBarAction::UpdateTitle(title) => assert_eq!(component_text(title, version), "Wither"),
        other => panic!("expected a title update, got {other:?}"),
    }
    assert_eq!(
        updates[2],
        BossBarAction::UpdateStyle {
            color: 1,
            division: 1
        }
    );
    assert_eq!(updates[3], BossBarAction::UpdateFlags(0x04));
    assert_eq!(updates[4], BossBarAction::Remove);

    assert!(matches!(
        handle.set_progress(0.1),
        Err(PlayerError::InvalidArgument(_))
    ));
    assert!(matches!(
        handle.hide(),
        Err(PlayerError::InvalidArgument(_))
    ));

    session.quit().await;
    proxy.wait_for_connection_count(0, T).await.unwrap();
    assert!(matches!(
        handle.set_progress(0.2),
        Err(PlayerError::Disconnected)
    ));

    proxy.shutdown().await.unwrap();
}

version_matrix!(boss_bar_is_added_updated_and_removed;
    p340 = 340, p764 = 764, p765 = 765, p766 = 766, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn boss_bars_need_1_9() {
    let (_a, _b, proxy, _recorder) = two_servers().await;
    let (_session, player) = join(&proxy, ProtocolVersion::V1_8).await;
    let refused = player.show_boss_bar(BossBar::new(Component::text("x")));
    assert!(matches!(refused, Err(PlayerError::Unsupported(_))));
    proxy.shutdown().await.unwrap();
}

async fn boss_bars_after_a_switch(version: ProtocolVersion) {
    let (backend_a, _b, proxy, _recorder) = two_servers().await;
    let (mut session, player) = join(&proxy, version).await;
    let mut conn_a = backend_a.next_connection(T).await.unwrap();

    let handle = player
        .show_boss_bar(BossBar::new(Component::text("Proxy bar")))
        .unwrap();
    session.expect::<CBossBar>(T).await.unwrap();

    let backend_bar = Uuid::from_u128(0xBAC0);
    conn_a
        .send_packet(&CBossBar {
            id: backend_bar,
            action: BossBarAction::Add {
                title: infrarust_test_harness::text::encode_component_json(
                    r#"{"text":"Backend bar"}"#,
                    version,
                )
                .unwrap(),
                health: 1.0,
                color: 0,
                division: 0,
                flags: 0,
            },
        })
        .await
        .unwrap();
    let forwarded = session.expect::<CBossBar>(T).await.unwrap();
    assert_eq!(forwarded.id, backend_bar);

    assert_eq!(
        player.connect(ServerId::new("b")).await.unwrap(),
        ConnectionResult::Success
    );
    let frames = frames_until_marker(&mut session, player.as_ref(), "switched").await;
    let bars = boss_bars(&frames, version);

    if version.less_than(ProtocolVersion::V1_20_2) {
        assert_eq!(
            bars,
            vec![CBossBar {
                id: backend_bar,
                action: BossBarAction::Remove,
            }],
            "the old server's bar is removed and the proxy bar stays"
        );
    } else {
        assert_eq!(bars.len(), 1, "{bars:?}");
        assert_eq!(bars[0].id, handle.id());
        assert!(
            matches!(bars[0].action, BossBarAction::Add { .. }),
            "the proxy bar is shown again after the configuration phase: {bars:?}"
        );
    }

    proxy.shutdown().await.unwrap();
}

version_matrix!(boss_bars_after_a_switch; p340 = 340, p764 = 764, p774 = 774);

fn added_bar_titles(bars: &[CBossBar], version: ProtocolVersion) -> Vec<(Uuid, String)> {
    bars.iter()
        .filter_map(|bar| match &bar.action {
            BossBarAction::Add { title, .. } => Some((bar.id, component_text(title, version))),
            _ => None,
        })
        .collect()
}

async fn presentation_survives_a_backend_reconfiguration(version: ProtocolVersion) {
    let backend = FakeBackend::builder().spawn().await.unwrap();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .start()
        .await
        .unwrap();
    let (mut session, player) = join(&proxy, version).await;
    let mut conn = backend.next_connection(T).await.unwrap();

    player
        .set_player_list_header_footer(Component::text("Top"), Component::text("Bottom"))
        .unwrap();
    let kept = player
        .show_boss_bar(BossBar::new(Component::text("Kept")))
        .unwrap();
    let hidden = player
        .show_boss_bar(BossBar::new(Component::text("Hidden")))
        .unwrap();
    let renamed = player
        .show_boss_bar(BossBar::new(Component::text("Before")))
        .unwrap();
    hidden.hide().unwrap();
    frames_until_marker(&mut session, player.as_ref(), "shown").await;

    conn.reconfigure(T).await.unwrap();
    renamed.set_title(Component::text("During")).unwrap();
    player
        .set_player_list_header_footer(Component::text("New top"), Component::text("Bottom"))
        .unwrap();
    plugin_message::send_to_client(&mut conn, "harness:sync", b"sync".to_vec())
        .await
        .unwrap();
    session
        .expect_config::<CConfigPluginMessage>(T)
        .await
        .unwrap();
    conn.finish_config(T).await.unwrap();
    session.expect_join(T).await.unwrap();

    let frames = frames_until_marker(&mut session, player.as_ref(), "reconfigured").await;
    let headers: Vec<(String, String)> = frames
        .iter()
        .filter(|frame| wire::is::<CTabListHeaderFooter>(frame, version))
        .map(|frame| {
            header_footer_text(
                &wire::decode::<CTabListHeaderFooter>(frame, version).unwrap(),
                version,
            )
        })
        .collect();
    assert_eq!(
        headers,
        vec![("New top".to_string(), "Bottom".to_string())],
        "the player list header and footer are shown again, once"
    );
    let bars = boss_bars(&frames, version);
    assert_eq!(
        added_bar_titles(&bars, version),
        vec![
            (kept.id(), "Kept".to_string()),
            (renamed.id(), "During".to_string()),
        ],
        "every live bar is shown again, once, and the hidden one is not: {bars:?}"
    );
    assert_eq!(bars.len(), 2, "{bars:?}");

    session.chat("still here").await.unwrap();
    assert_eq!(
        conn.expect::<SChatMessage>(T).await.unwrap().message,
        "still here"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(presentation_survives_a_backend_reconfiguration;
    p764 = 764, p765 = 765, p766 = 766, p770 = 770, p774 = 774, p776 = 776);

fn pack_request() -> ResourcePackRequest {
    ResourcePackRequest::new(PACK_URL)
        .id(Uuid::from_u128(0x9ACC))
        .hash(HASH)
        .required(true)
        .prompt(Component::text("Please"))
}

async fn statuses(recorder: &Recorder, origin: &str, count: usize) -> Vec<Recorded> {
    recorder
        .wait_for(
            |e| {
                e.kind == EventKind::PlayerResourcePackStatus
                    && e.detail["origin"] == origin
                    && e.detail["status"] == "successfully_loaded"
            },
            T,
        )
        .await
        .unwrap();
    let seen = recorder
        .filter(|e| e.kind == EventKind::PlayerResourcePackStatus && e.detail["origin"] == origin);
    assert_eq!(seen.len(), count, "{seen:#?}");
    seen
}

fn assert_proxy_statuses(seen: &[Recorded], pack: &ResourcePackRequest) {
    let pack_id = pack.id.to_string();
    let statuses: Vec<&str> = seen
        .iter()
        .map(|e| e.detail["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, ["accepted", "successfully_loaded"]);
    for event in seen {
        assert_eq!(event.detail["pack_id"], pack_id.as_str(), "{event:?}");
    }
}

async fn backend_frames_until_chat(conn: &mut BackendConn, marker: &str) -> Vec<PacketFrame> {
    let version = conn.version();
    let mut seen = Vec::new();
    loop {
        let frame = conn.recv_frame(T).await.unwrap();
        if infrarust_test_harness::ChatFrame::classify(&frame, version)
            .unwrap()
            .is_some_and(|chat| chat.is_message(marker))
        {
            return seen;
        }
        seen.push(frame);
    }
}

async fn resource_pack_round_trip(version: ProtocolVersion) {
    let (backend_a, _b, proxy, recorder) = two_servers().await;
    let mut session = proxy
        .client(version)
        .login("Steve")
        .await
        .unwrap()
        .joined()
        .unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();
    let mut conn = backend_a.next_connection(T).await.unwrap();
    let pack = pack_request();

    player.send_resource_pack(pack.clone()).unwrap();
    let modern = version.no_less_than(ProtocolVersion::V1_20_3);
    if modern {
        let push = session.expect::<CResourcePackPush>(T).await.unwrap();
        assert_eq!(push.id, pack.id);
        assert_eq!(push.url, PACK_URL);
        assert_eq!(push.hash, HASH);
        assert!(push.forced);
        assert_eq!(component_text(&push.prompt.unwrap(), version), "Please");
    } else {
        let sent = session.expect::<CResourcePack>(T).await.unwrap();
        assert_eq!(sent.url, PACK_URL);
        assert_eq!(sent.hash, HASH);
        if version.no_less_than(ProtocolVersion::V1_17) {
            assert!(sent.forced);
            assert_eq!(component_text(&sent.prompt.unwrap(), version), "Please");
        } else {
            assert!(!sent.forced);
            assert_eq!(sent.prompt, None);
        }
    }

    for result in [
        ResourcePackResult::Accepted,
        ResourcePackResult::SuccessfullyLoaded,
    ] {
        session
            .send_packet(&SResourcePackResponse {
                id: Some(pack.id),
                hash: Some(HASH.to_string()),
                result,
            })
            .await
            .unwrap();
    }
    session.chat("after the pack").await.unwrap();
    let reached = backend_frames_until_chat(&mut conn, "after the pack").await;
    assert!(
        !reached
            .iter()
            .any(|frame| wire::is::<SResourcePackResponse>(frame, version)),
        "the proxy's pack answers must not reach the backend"
    );
    assert_proxy_statuses(&statuses(&recorder, "proxy", 2).await, &pack);

    if modern {
        player.remove_resource_pack(Some(pack.id)).unwrap();
        let pop = session.expect::<CResourcePackPop>(T).await.unwrap();
        assert_eq!(pop.id, Some(pack.id));
        player.remove_resource_pack(None).unwrap();
        let pop_all = session.expect::<CResourcePackPop>(T).await.unwrap();
        assert_eq!(pop_all.id, None);
    } else {
        assert!(matches!(
            player.remove_resource_pack(Some(pack.id)),
            Err(PlayerError::Unsupported(_))
        ));
    }

    proxy.shutdown().await.unwrap();
}

version_matrix!(resource_pack_round_trip;
    p47 = 47, p340 = 340, p764 = 764, p765 = 765, p766 = 766, p774 = 774);

async fn backend_resource_packs_pass_through(version: ProtocolVersion) {
    let (backend_a, _b, proxy, recorder) = two_servers().await;
    let (mut session, _player) = join(&proxy, version).await;
    let mut conn = backend_a.next_connection(T).await.unwrap();
    let backend_pack = Uuid::from_u128(0xB0B);

    if version.no_less_than(ProtocolVersion::V1_20_3) {
        conn.send_packet(&CResourcePackPush {
            id: backend_pack,
            url: PACK_URL.to_string(),
            hash: HASH.to_string(),
            forced: false,
            prompt: None,
        })
        .await
        .unwrap();
        session.expect::<CResourcePackPush>(T).await.unwrap();
    } else {
        conn.send_packet(&CResourcePack {
            url: PACK_URL.to_string(),
            hash: HASH.to_string(),
            forced: false,
            prompt: None,
        })
        .await
        .unwrap();
        session.expect::<CResourcePack>(T).await.unwrap();
    }

    for expected in [
        ResourcePackResult::Accepted,
        ResourcePackResult::SuccessfullyLoaded,
    ] {
        let answer = conn.expect::<SResourcePackResponse>(T).await.unwrap();
        assert_eq!(answer.result, expected);
        if version.no_less_than(ProtocolVersion::V1_20_3) {
            assert_eq!(answer.id, Some(backend_pack));
        }
    }
    let seen = statuses(&recorder, "backend", 2).await;
    let expected_id = if version.no_less_than(ProtocolVersion::V1_20_3) {
        serde_json::json!(backend_pack.to_string())
    } else {
        serde_json::Value::Null
    };
    assert!(
        seen.iter().all(|e| e.detail["pack_id"] == expected_id),
        "{seen:#?}"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(backend_resource_packs_pass_through; p340 = 340, p764 = 764, p765 = 765, p774 = 774);

async fn resource_packs_in_the_configuration_phase(version: ProtocolVersion) {
    let backend = FakeBackend::builder().hold_config().spawn().await.unwrap();
    let recorder = Recorder::new();
    let proxy = TestProxy::builder()
        .server(ServerSpec::offline("lobby").backend(backend.addr()))
        .plugin(recorder.plugin())
        .start()
        .await
        .unwrap();

    let client = proxy.client(version).answer_resource_packs([
        ResourcePackResult::Accepted,
        ResourcePackResult::SuccessfullyLoaded,
    ]);
    let login = tokio::spawn(async move { client.login("Steve").await });
    let mut conn = backend.next_connection(T).await.unwrap();
    let player = proxy.wait_for_player("Steve", T).await.unwrap();

    let pack = pack_request();
    player.send_resource_pack(pack.clone()).unwrap();
    assert_proxy_statuses(&statuses(&recorder, "proxy", 2).await, &pack);

    let backend_pack = Uuid::from_u128(0xB0B);
    let legacy = version.less_than(ProtocolVersion::V1_20_3);
    if legacy {
        conn.send_packet(&CConfigResourcePack {
            url: PACK_URL.to_string(),
            hash: HASH.to_string(),
            forced: false,
            prompt: None,
        })
        .await
        .unwrap();
    } else {
        conn.send_packet(&CConfigResourcePackPush {
            id: backend_pack,
            url: PACK_URL.to_string(),
            hash: HASH.to_string(),
            forced: false,
            prompt: None,
        })
        .await
        .unwrap();
    }
    for expected in [
        ResourcePackResult::Accepted,
        ResourcePackResult::SuccessfullyLoaded,
    ] {
        let answer = conn.expect::<SConfigResourcePackResponse>(T).await.unwrap();
        assert_eq!(answer.result, expected);
        if !legacy {
            assert_eq!(answer.id, Some(backend_pack));
        }
    }
    statuses(&recorder, "backend", 2).await;

    conn.finish_config(T).await.unwrap();
    let outcome = tokio::time::timeout(T, login)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(outcome, LoginOutcome::Joined(_)));
    let answers = conn
        .received()
        .iter()
        .filter(|frame| wire::is::<SConfigResourcePackResponse>(frame, version))
        .count();
    assert_eq!(
        answers, 2,
        "only the backend's pack is answered to the backend"
    );

    proxy.shutdown().await.unwrap();
}

version_matrix!(resource_packs_in_the_configuration_phase; p764 = 764, p765 = 765, p774 = 774);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_resource_packs_are_refused() {
    let (_a, _b, proxy, _recorder) = two_servers().await;
    let (_session, player) = join(&proxy, ProtocolVersion(774)).await;
    let refused = player.send_resource_pack(ResourcePackRequest::new(PACK_URL).hash("nope"));
    assert!(matches!(refused, Err(PlayerError::InvalidArgument(_))));
    proxy.shutdown().await.unwrap();
}
