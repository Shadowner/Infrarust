use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::player::{BossBarUpdate, Player};
use infrarust_api::test_util::MockPlayer;
use infrarust_api::types::Component;

use crate::bindings::infrarust::plugin::{players, types as wt};
use crate::deadline::Deadline;
use crate::store_state::PluginStoreState;

use super::{RecordingPlayer, address, stalled_player, state_with, text_of};

#[tokio::test]
async fn players_are_read_by_id_name_and_uuid() {
    let steve = MockPlayer::new(7, "Steve").on_server("lobby").into_arc();
    let uuid = steve.profile().uuid;
    let mut state = state_with(CapabilitySet::baseline(), vec![steve]);

    let info = players::Host::get(&mut state, 7)
        .await
        .unwrap()
        .expect("online");
    assert_eq!(info.player.username, "Steve");
    assert_eq!(info.current_server.as_deref(), Some("lobby"));
    let by_name = players::Host::get_by_name(&mut state, "Steve".into())
        .await
        .unwrap();
    assert_eq!(by_name.map(|info| info.player.id), Some(7));
    let by_uuid = players::Host::get_by_uuid(&mut state, crate::convert::uuid_to_wit(uuid))
        .await
        .unwrap();
    assert_eq!(by_uuid.map(|info| info.player.id), Some(7));
    let unknown = players::Host::get_by_uuid(
        &mut state,
        crate::convert::uuid_to_wit(uuid::Uuid::from_u128(9)),
    )
    .await
    .unwrap();
    assert!(unknown.is_none());
    assert_eq!(players::Host::count(&mut state, None).await.unwrap(), 1);
    assert_eq!(
        players::Host::list(&mut state, Some("survival".into()))
            .await
            .unwrap()
            .len(),
        0
    );
    let listed = players::Host::list(&mut state, Some("lobby".into()))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].player.id, 7);
    assert_eq!(listed[0].player.username, "Steve");
    assert_eq!(
        listed[0].player.uuid,
        crate::convert::uuid_to_wit(uuid),
        "a listed player carries the same reference as its full record"
    );
    assert_eq!(listed[0].current_server.as_deref(), Some("lobby"));
    assert_eq!(players::Host::list(&mut state, None).await.unwrap(), listed);
}

fn connected_from(id: u16, username: &str, ip: &str) -> Arc<dyn Player> {
    MockPlayer::new(id.into(), username)
        .with_remote_addr(SocketAddr::new(ip.parse().unwrap(), 40_000 + id))
        .on_server("lobby")
        .into_arc()
}

async fn players_at(state: &mut PluginStoreState, ip: &str) -> Vec<u64> {
    let ip = crate::convert::ip_to_wit(ip.parse().unwrap());
    let found = players::Host::get_by_ip(state, ip).await.unwrap();
    let listed = players::Host::list(state, None).await.unwrap();
    assert!(
        found.iter().all(|summary| listed.contains(summary)),
        "an address lookup answers the records list answers: {found:?}"
    );
    let mut ids: Vec<u64> = found.iter().map(|summary| summary.player.id).collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn players_are_found_by_the_ip_they_connected_from() {
    let mut state = state_with(
        CapabilitySet::baseline(),
        vec![
            connected_from(7, "Steve", "203.0.113.7"),
            connected_from(8, "SteveAlt", "203.0.113.7"),
            connected_from(9, "Alex", "198.51.100.2"),
            connected_from(10, "Herobrine", "2001:db8::7"),
        ],
    );

    assert_eq!(players_at(&mut state, "203.0.113.7").await, [7, 8]);
    assert_eq!(players_at(&mut state, "::ffff:203.0.113.7").await, [7, 8]);
    assert_eq!(players_at(&mut state, "198.51.100.2").await, [9]);
    assert_eq!(players_at(&mut state, "2001:db8::7").await, [10]);
    assert!(players_at(&mut state, "192.0.2.1").await.is_empty());
    assert!(players_at(&mut state, "2001:db8::8").await.is_empty());

    let alex = players::Host::get_by_ip(&mut state, wt::IpAddress::Ipv4((198, 51, 100, 2)))
        .await
        .unwrap();
    assert_eq!(alex[0].player.username, "Alex");
    assert_eq!(alex[0].current_server.as_deref(), Some("lobby"));
}

#[tokio::test]
async fn an_ip_lookup_without_player_read_answers_like_list() {
    let mut state = state_with(
        CapabilitySet::baseline().without(Capability::PlayerRead),
        vec![connected_from(7, "Steve", "203.0.113.7")],
    );
    let found = players::Host::get_by_ip(&mut state, wt::IpAddress::Ipv4((203, 0, 113, 7)))
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(found, players::Host::list(&mut state, None).await.unwrap());
}

#[tokio::test]
async fn a_message_reaches_the_player_by_id() {
    let steve = MockPlayer::new(7, "Steve").into_arc();
    let mut state = state_with(CapabilitySet::baseline(), vec![steve.clone()]);

    let sent = players::Host::send_message(&mut state, 7, text_of("hi"))
        .await
        .unwrap();
    assert_eq!(sent, Ok(()));
    assert_eq!(steve.messages(), [Component::text("hi")]);
}

#[tokio::test]
async fn an_offline_player_is_player_gone_and_a_bad_text_is_an_argument_error() {
    let steve = MockPlayer::new(7, "Steve").into_arc();
    let mut state = state_with(CapabilitySet::baseline(), vec![steve]);

    let gone = players::Host::send_message(&mut state, 8, text_of("hi"))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.kind, wt::ErrorKind::PlayerGone);

    let broken = wt::Component { nodes: Vec::new() };
    let invalid = players::Host::send_action_bar(&mut state, 7, broken)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(invalid.kind, wt::ErrorKind::InvalidArgument);
}

#[tokio::test]
async fn a_disconnect_still_pending_is_dropped_with_the_host_resources() {
    let player = stalled_player();
    let mut state = state_with(CapabilitySet::baseline(), vec![Arc::clone(&player)]);
    let idle = Arc::strong_count(&player);

    players::Host::disconnect(&mut state, 1, text_of("bye"))
        .await
        .unwrap()
        .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(Arc::strong_count(&player), idle + 1);

    state.release_host_resources();
    tokio::task::yield_now().await;
    assert_eq!(Arc::strong_count(&player), idle);
}

#[tokio::test]
async fn a_disconnect_that_completes_at_once_outlives_the_host_resources() {
    let player: Arc<dyn Player> = MockPlayer::new(1, "tester").into_arc();
    let mut state = state_with(CapabilitySet::baseline(), vec![Arc::clone(&player)]);

    players::Host::disconnect(&mut state, 1, text_of("bye"))
        .await
        .unwrap()
        .unwrap();
    state.release_host_resources();

    assert!(!player.is_connected());
}

#[tokio::test]
async fn switch_server_gives_up_long_before_the_host_call_timeout() {
    let mut state = state_with(CapabilitySet::baseline(), vec![stalled_player()]);

    let started = Instant::now();
    let result = players::Host::switch_server(&mut state, 1, "lobby".to_owned())
        .await
        .expect("a saturated session is a host-error, not a trap");
    let elapsed = started.elapsed();

    assert!(
        matches!(&result, Err(e) if e.kind == wt::ErrorKind::Timeout),
        "{result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "held the instance lock for {elapsed:?}"
    );
}

#[tokio::test]
async fn switch_server_stops_at_the_call_deadline_when_that_comes_first() {
    let mut state = state_with(CapabilitySet::baseline(), vec![stalled_player()]);
    state.begin_call(Some(Deadline::after(Duration::ZERO)));

    let result = players::Host::switch_server(&mut state, 1, "lobby".to_owned())
        .await
        .expect("a call out of time is a host-error, not a trap");

    assert!(
        matches!(&result, Err(e) if e.kind == wt::ErrorKind::Timeout && e.message.contains("deadline")),
        "{result:?}"
    );
}

#[tokio::test]
async fn player_write_calls_are_refused_without_the_capability() {
    let mut state = state_with(
        CapabilitySet::baseline().without(Capability::PlayerWrite),
        vec![stalled_player()],
    );
    let denied = |result: Result<(), wt::HostError>| matches!(&result, Err(e) if e.kind == wt::ErrorKind::PermissionDenied && e.message.contains("player-write"));

    assert!(denied(
        players::Host::switch_server(&mut state, 1, "lobby".to_owned())
            .await
            .unwrap()
    ));
    assert!(denied(
        players::Host::send_message(&mut state, 1, text_of("hi"))
            .await
            .unwrap()
    ));
    assert!(denied(
        players::Host::disconnect(&mut state, 1, text_of("bye"))
            .await
            .unwrap()
    ));
}

pub(super) fn boss_bar() -> players::BossBar {
    players::BossBar {
        title: text_of("boss"),
        progress: 0.25,
        color: players::BossBarColor::Red,
        overlay: players::BossBarOverlay::Notched10,
        flags: players::BossBarFlags::DARKEN_SCREEN,
    }
}

pub(super) fn resource_pack() -> players::ResourcePackRequest {
    players::ResourcePackRequest {
        id: wt::Uuid { hi: 0, lo: 7 },
        url: "https://example.com/pack.zip".into(),
        hash: None,
        required: true,
        prompt: Some(text_of("please")),
    }
}

#[tokio::test]
async fn player_info_carries_settings_and_known_channels() {
    let (player, _) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);
    let info = players::Host::get(&mut state, 1).await.unwrap().unwrap();
    assert_eq!(info.known_channels, ["test:echo"]);
    assert_eq!(info.settings.map(|s| s.locale).as_deref(), Some("fr_fr"));
}

#[tokio::test]
async fn a_boss_bar_is_shown_updated_and_hidden_by_its_id() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);

    let bar = players::Host::show_boss_bar(&mut state, 1, boss_bar())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        crate::convert::uuid_from_wit(bar),
        uuid::Uuid::from_u128(42)
    );
    players::Host::update_boss_bar(&mut state, bar, players::BossBarUpdate::Progress(0.75))
        .await
        .unwrap()
        .unwrap();
    players::Host::hide_boss_bar(&mut state, bar)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        *seen.bars.lock().unwrap(),
        [
            (
                uuid::Uuid::from_u128(42),
                Some(BossBarUpdate::Progress(0.75))
            ),
            (uuid::Uuid::from_u128(42), None),
        ]
    );
    let gone = players::Host::hide_boss_bar(&mut state, bar)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(gone.kind, wt::ErrorKind::NotFound);
}

#[tokio::test]
async fn cookies_round_trip_and_connect_reports_the_switch_outcome() {
    let (player, seen) = RecordingPlayer::shared();
    let mut state = state_with(CapabilitySet::baseline(), vec![player]);

    players::Host::store_cookie(&mut state, 1, "infrarust:token".into(), b"t".to_vec())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        players::Host::request_cookie(&mut state, 1, "infrarust:token".into())
            .await
            .unwrap(),
        Ok(Some(b"t".to_vec()))
    );
    assert_eq!(
        players::Host::connect(&mut state, 1, "lobby".into())
            .await
            .unwrap(),
        Ok(players::ConnectionResult::AlreadyConnected)
    );
    assert_eq!(
        players::Host::connect(&mut state, 1, "full".into())
            .await
            .unwrap(),
        Ok(players::ConnectionResult::Denied(text_of("full")))
    );

    players::Host::send_resource_pack(&mut state, 1, resource_pack())
        .await
        .unwrap()
        .unwrap();
    let pack = seen.packs.lock().unwrap()[0].clone();
    assert_eq!(pack.id, uuid::Uuid::from_u128(7));
    assert_eq!(pack.prompt, Some(Component::text("please")));

    let mut bad = resource_pack();
    bad.hash = Some("not-a-sha1".into());
    let refused = players::Host::send_resource_pack(&mut state, 1, bad)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.kind, wt::ErrorKind::InvalidArgument);

    let unsupported = players::Host::transfer(&mut state, 1, address())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(unsupported.kind, wt::ErrorKind::Unsupported);
}
