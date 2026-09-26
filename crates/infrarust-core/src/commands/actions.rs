use std::fmt;
use std::sync::Arc;

use infrarust_api::player::Player;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, ServerId};

pub const DEFAULT_KICK_REASON: &str = "Kicked by an administrator";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    PlayerOffline(String),
    PlayerPassive(String),
    UnknownServer(String),
    Switch {
        player: String,
        server: String,
        error: String,
    },
    EmptyMessage,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlayerOffline(name) => write!(f, "Player '{name}' is not online"),
            Self::PlayerPassive(name) => write!(
                f,
                "Player '{name}' is on a passive proxy path and cannot be acted on by the proxy"
            ),
            Self::UnknownServer(name) => write!(f, "Server '{name}' not found"),
            Self::Switch {
                player,
                server,
                error,
            } => write!(f, "Failed to send {player} to {server}: {error}"),
            Self::EmptyMessage => f.write_str("No message provided"),
        }
    }
}

impl std::error::Error for ActionError {}

#[derive(Debug)]
pub struct Kicked {
    pub player: String,
    pub server: Option<ServerId>,
    pub reason: String,
}

#[derive(Debug)]
pub struct Sent {
    pub player: String,
    pub server: ServerId,
}

#[derive(Debug)]
pub struct Broadcast {
    pub recipients: usize,
    pub scope: Option<ServerId>,
}

pub fn find_player(
    players: &dyn PlayerRegistry,
    name: &str,
) -> Result<Arc<dyn Player>, ActionError> {
    players
        .get_player(name)
        .ok_or_else(|| ActionError::PlayerOffline(name.to_string()))
}

fn known_server(configs: &dyn ConfigService, name: &str) -> Result<ServerId, ActionError> {
    let server = ServerId::new(name);
    if configs.get_server_config(&server).is_none() {
        return Err(ActionError::UnknownServer(name.to_string()));
    }
    Ok(server)
}

pub async fn kick_player(
    players: &dyn PlayerRegistry,
    name: &str,
    reason: Option<String>,
) -> Result<Kicked, ActionError> {
    let player = find_player(players, name)?;
    let reason = reason.unwrap_or_else(|| DEFAULT_KICK_REASON.to_string());
    let server = player.current_server();
    tracing::info!(
        player = %player.profile().username,
        reason = %reason,
        server = server.as_ref().map_or("-", ServerId::as_str),
        "player kicked by an administrator"
    );
    player.disconnect(Component::text(&reason)).await;
    Ok(Kicked {
        player: player.profile().username.clone(),
        server,
        reason,
    })
}

pub async fn send_player(
    players: &dyn PlayerRegistry,
    configs: &dyn ConfigService,
    name: &str,
    server: &str,
) -> Result<Sent, ActionError> {
    let player = find_player(players, name)?;
    let target = known_server(configs, server)?;
    if !player.is_active() {
        return Err(ActionError::PlayerPassive(name.to_string()));
    }
    player
        .switch_server(target.clone())
        .await
        .map_err(|error| ActionError::Switch {
            player: name.to_string(),
            server: server.to_string(),
            error: error.to_string(),
        })?;
    tracing::info!(
        player = %player.profile().username,
        server = target.as_str(),
        "player transferred by an administrator"
    );
    Ok(Sent {
        player: player.profile().username.clone(),
        server: target,
    })
}

pub fn broadcast(
    players: &dyn PlayerRegistry,
    configs: &dyn ConfigService,
    scope: Option<&str>,
    message: &str,
) -> Result<Broadcast, ActionError> {
    if message.trim().is_empty() {
        return Err(ActionError::EmptyMessage);
    }
    let scope = scope.map(|name| known_server(configs, name)).transpose()?;
    let recipients = match &scope {
        Some(server) => players.get_players_on_server(server),
        None => players.get_all_players(),
    };
    let component = Component::from_legacy(message);
    let delivered = recipients
        .iter()
        .filter(|player| player.send_message(component.clone()).is_ok())
        .count();
    tracing::info!(
        message,
        recipients = delivered,
        scope = scope.as_ref().map_or("-", ServerId::as_str),
        "broadcast sent by an administrator"
    );
    Ok(Broadcast {
        recipients: delivered,
        scope,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_api::test_util::{MockConfigService, MockPlayer, MockPlayerRegistry};

    use super::*;

    fn world() -> (
        MockPlayerRegistry,
        MockConfigService,
        Arc<MockPlayer>,
        Arc<MockPlayer>,
    ) {
        let steve = MockPlayer::new(1, "Steve").into_arc();
        let alex = MockPlayer::new(2, "Alex").passive().into_arc();
        let players = MockPlayerRegistry::new()
            .with(Arc::clone(&steve))
            .with(Arc::clone(&alex));
        let configs = MockConfigService::new().with_server(MockConfigService::server("lobby"));
        (players, configs, steve, alex)
    }

    #[test]
    fn find_player_reports_offline_players() {
        let (players, _, _, _) = world();
        assert!(find_player(&players, "Steve").is_ok());
        assert_eq!(
            find_player(&players, "Nobody").err(),
            Some(ActionError::PlayerOffline("Nobody".into()))
        );
    }

    #[tokio::test]
    async fn kick_without_a_reason_uses_the_shared_default() {
        let (players, _, steve, alex) = world();
        let kicked = kick_player(&players, "Steve", None).await.unwrap();
        assert_eq!(kicked.reason, DEFAULT_KICK_REASON);
        assert_eq!(steve.kicks()[0].to_plain(), DEFAULT_KICK_REASON);

        let kicked = kick_player(&players, "Alex", Some("bye".into()))
            .await
            .unwrap();
        assert_eq!(kicked.reason, "bye");
        assert_eq!(alex.kicks()[0].to_plain(), "bye");
    }

    #[tokio::test]
    async fn send_checks_the_player_then_the_server_then_the_proxy_path() {
        let (players, configs, steve, alex) = world();
        assert_eq!(
            send_player(&players, &configs, "Nobody", "lobby")
                .await
                .unwrap_err(),
            ActionError::PlayerOffline("Nobody".into())
        );
        assert_eq!(
            send_player(&players, &configs, "Steve", "nowhere")
                .await
                .unwrap_err(),
            ActionError::UnknownServer("nowhere".into())
        );
        assert_eq!(
            send_player(&players, &configs, "Alex", "lobby")
                .await
                .unwrap_err(),
            ActionError::PlayerPassive("Alex".into())
        );
        assert!(alex.switches().is_empty());

        let sent = send_player(&players, &configs, "Steve", "lobby")
            .await
            .unwrap();
        assert_eq!(sent.server, ServerId::new("lobby"));
        assert_eq!(steve.switches(), [ServerId::new("lobby")]);
    }

    #[test]
    fn broadcast_counts_only_delivered_messages_and_checks_the_scope() {
        let (players, configs, steve, alex) = world();
        assert_eq!(
            broadcast(&players, &configs, None, "  ").unwrap_err(),
            ActionError::EmptyMessage
        );
        assert_eq!(
            broadcast(&players, &configs, Some("nowhere"), "hi").unwrap_err(),
            ActionError::UnknownServer("nowhere".into())
        );
        let sent = broadcast(&players, &configs, None, "hi").unwrap();
        assert_eq!(sent.recipients, 1);
        assert_eq!(steve.sent_text(), "hi");
        assert!(alex.messages().is_empty());
    }
}
