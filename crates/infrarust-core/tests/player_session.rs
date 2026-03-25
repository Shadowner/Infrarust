#![allow(clippy::unwrap_used, clippy::expect_used)]

use infrarust_api::player::Player;
use infrarust_api::types::{Component, ServerId};
use infrarust_core::player::{PlayerCommand, PlayerSession};

#[test]
fn test_send_message_active_player() {
    let (session, mut rx) = PlayerSession::new_test(true);

    session
        .send_message(Component::text("hello"))
        .expect("should succeed");

    let cmd = rx.try_recv().expect("should have a command");
    assert!(matches!(cmd, PlayerCommand::SendMessage(_)));
}

#[test]
fn test_send_message_passive_player() {
    let (session, _rx) = PlayerSession::new_test(false);

    let result = session.send_message(Component::text("hello"));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        infrarust_api::error::PlayerError::NotActive
    ));
}

#[test]
fn test_send_message_disconnected() {
    let (session, _rx) = PlayerSession::new_test(true);
    session.set_disconnected();

    let result = session.send_message(Component::text("hello"));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        infrarust_api::error::PlayerError::Disconnected
    ));
}

#[test]
fn test_channel_backpressure() {
    let (session, _rx) = PlayerSession::new_test(true);

    // Fill the channel (capacity = 32)
    for i in 0..32 {
        session
            .send_message(Component::text(format!("msg {i}")))
            .expect("should succeed");
    }

    // 33rd should fail
    let result = session.send_message(Component::text("overflow"));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        infrarust_api::error::PlayerError::SendFailed(_)
    ));
}

#[tokio::test]
async fn test_kick_cancels_token() {
    let (session, _rx) = PlayerSession::new_test(true);
    assert!(!session.shutdown_token().is_cancelled());

    session.disconnect(Component::text("goodbye")).await;

    assert!(session.shutdown_token().is_cancelled());
}

#[test]
fn test_current_server_update() {
    let (session, _rx) = PlayerSession::new_test(true);
    assert!(session.current_server().is_none());

    session.set_current_server(ServerId::new("lobby"));
    assert_eq!(session.current_server().unwrap().as_str(), "lobby");

    session.set_current_server(ServerId::new("survival"));
    assert_eq!(session.current_server().unwrap().as_str(), "survival");
}
