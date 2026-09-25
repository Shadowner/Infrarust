use std::collections::VecDeque;

use tokio::sync::mpsc;

use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::registry::PacketRegistry;

use super::PlayerCommand;
use super::packets;
use crate::error::CoreError;
use crate::session::client_bridge::ClientBridge;

#[derive(Debug)]
pub(crate) enum CommandOutcome {
    Continue,
    Kick(Component),
    Switch(ServerId),
}

pub(crate) struct CommandInbox {
    receiver: mpsc::Receiver<PlayerCommand>,
    deferred: VecDeque<PlayerCommand>,
}

impl CommandInbox {
    pub(crate) const fn new(receiver: mpsc::Receiver<PlayerCommand>) -> Self {
        Self {
            receiver,
            deferred: VecDeque::new(),
        }
    }

    pub(crate) async fn recv(&mut self) -> Option<PlayerCommand> {
        self.receiver.recv().await
    }

    pub(crate) fn apply(
        &mut self,
        command: PlayerCommand,
        client: &mut ClientBridge,
        registry: &PacketRegistry,
        in_game: bool,
    ) -> CommandOutcome {
        if let PlayerCommand::Kick(reason) = command {
            return CommandOutcome::Kick(reason);
        }
        self.deferred.push_back(command);
        if in_game {
            self.release(client, registry)
        } else {
            CommandOutcome::Continue
        }
    }

    pub(crate) fn drain(
        &mut self,
        client: &mut ClientBridge,
        registry: &PacketRegistry,
        in_game: bool,
    ) -> CommandOutcome {
        if in_game {
            let outcome = self.release(client, registry);
            if !matches!(outcome, CommandOutcome::Continue) {
                return outcome;
            }
        }
        while let Ok(command) = self.receiver.try_recv() {
            let outcome = self.apply(command, client, registry, in_game);
            if !matches!(outcome, CommandOutcome::Continue) {
                return outcome;
            }
        }
        CommandOutcome::Continue
    }

    pub(crate) fn take_kick(
        &mut self,
        client: &mut ClientBridge,
        registry: &PacketRegistry,
        in_game: bool,
    ) -> Option<Component> {
        loop {
            match self.drain(client, registry, in_game) {
                CommandOutcome::Kick(reason) => return Some(reason),
                CommandOutcome::Switch(_) => {}
                CommandOutcome::Continue => return None,
            }
        }
    }

    fn release(&mut self, client: &mut ClientBridge, registry: &PacketRegistry) -> CommandOutcome {
        while let Some(command) = self.deferred.pop_front() {
            match command {
                PlayerCommand::Kick(reason) => return CommandOutcome::Kick(reason),
                PlayerCommand::SwitchServer(target) => return CommandOutcome::Switch(target),
                other => {
                    if let Err(e) = queue_frames(client, other, registry) {
                        tracing::warn!("failed to deliver player command: {e}");
                    }
                }
            }
        }
        CommandOutcome::Continue
    }
}

fn queue_frames(
    client: &mut ClientBridge,
    command: PlayerCommand,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let version = client.protocol_version;
    match command {
        PlayerCommand::SendMessage(message) => client.queue_frame(
            &packets::build_system_chat_message(&message, version, registry)?,
        ),
        PlayerCommand::SendActionBar(message) => {
            client.queue_frame(&packets::build_action_bar(&message, version, registry)?)
        }
        PlayerCommand::SendTitle(title) => {
            for frame in packets::build_title_packets(&title, version, registry)? {
                client.queue_frame(&frame)?;
            }
            Ok(())
        }
        PlayerCommand::SendPacket(raw) => {
            client.queue_frame(&PacketFrame::new(raw.packet_id, raw.data))
        }
        PlayerCommand::Kick(_) | PlayerCommand::SwitchServer(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use infrarust_protocol::version::ProtocolVersion;

    use super::*;
    use crate::limbo::test_helpers::{test_client_bridge, test_registry};
    use crate::player::PlayerSession;

    fn inbox() -> (mpsc::Sender<PlayerCommand>, CommandInbox) {
        let (tx, rx) = PlayerSession::channel();
        (tx, CommandInbox::new(rx))
    }

    fn message(text: &str) -> PlayerCommand {
        PlayerCommand::SendMessage(Component::text(text))
    }

    #[tokio::test]
    async fn commands_wait_for_the_game_and_keep_their_order() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = test_registry();
        let (tx, mut inbox) = inbox();
        tx.try_send(message("first")).unwrap();
        tx.try_send(PlayerCommand::SwitchServer(ServerId::new("b")))
            .unwrap();
        tx.try_send(message("second")).unwrap();

        assert!(matches!(
            inbox.drain(&mut client, &registry, false),
            CommandOutcome::Continue
        ));
        assert_eq!(inbox.deferred.len(), 3);

        match inbox.drain(&mut client, &registry, true) {
            CommandOutcome::Switch(target) => assert_eq!(target, ServerId::new("b")),
            other => panic!("expected the deferred switch, got {other:?}"),
        }
        assert_eq!(inbox.deferred.len(), 1);
        assert!(matches!(
            inbox.drain(&mut client, &registry, true),
            CommandOutcome::Continue
        ));
        assert!(inbox.deferred.is_empty());
    }

    #[tokio::test]
    async fn a_kick_is_never_deferred() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = test_registry();
        let (_tx, mut inbox) = inbox();
        inbox.apply(message("queued"), &mut client, &registry, false);

        match inbox.apply(
            PlayerCommand::Kick(Component::text("bye")),
            &mut client,
            &registry,
            false,
        ) {
            CommandOutcome::Kick(reason) => assert_eq!(reason, Component::text("bye")),
            other => panic!("expected the kick, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn take_kick_looks_past_switch_requests() {
        let (mut client, _raw) = test_client_bridge(ProtocolVersion::V1_21).await;
        let registry = test_registry();
        let (tx, mut inbox) = inbox();
        tx.try_send(PlayerCommand::SwitchServer(ServerId::new("b")))
            .unwrap();
        tx.try_send(message("last words")).unwrap();
        tx.try_send(PlayerCommand::Kick(Component::text("bye")))
            .unwrap();

        assert_eq!(
            inbox.take_kick(&mut client, &registry, true),
            Some(Component::text("bye"))
        );
        assert_eq!(inbox.take_kick(&mut client, &registry, true), None);
    }
}
