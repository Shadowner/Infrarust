use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::mpsc;

use infrarust_api::events::connection::ConnectCause;
use infrarust_api::types::{Component, ProtocolVersion as ApiVersion, ServerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::cookie::{
    CConfigCookieRequest, CConfigStoreCookie, CCookieRequest, CStoreCookie,
};
use infrarust_protocol::packets::play::transfer::{CConfigTransfer, CTransfer};
use infrarust_protocol::packets::resource_pack::{
    CConfigResourcePack, CConfigResourcePackPop, CConfigResourcePackPush, CResourcePack,
    CResourcePackPop, CResourcePackPush,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use super::packets::{self, encode_packet};
use super::presentation::Presentation;
use super::{BossBarCommand, ClientCommand, MessageTarget, OutgoingMessage, PlayerCommand};
use crate::error::CoreError;
use crate::plugin_messaging::channels::{self, MessageIds};
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::util::text::encode_text_component;

#[derive(Debug)]
pub(crate) enum CommandOutcome {
    Continue,
    Kick(Component),
    Switch(ServerId, ConnectCause),
}

pub(crate) struct CommandInbox {
    receiver: mpsc::Receiver<PlayerCommand>,
    deferred: VecDeque<PlayerCommand>,
    messages: VecDeque<OutgoingMessage>,
    client_commands: VecDeque<ClientCommand>,
    presentation: Option<Arc<Presentation>>,
}

impl CommandInbox {
    pub(crate) const fn new(receiver: mpsc::Receiver<PlayerCommand>) -> Self {
        Self {
            receiver,
            deferred: VecDeque::new(),
            messages: VecDeque::new(),
            client_commands: VecDeque::new(),
            presentation: None,
        }
    }

    #[must_use]
    pub(crate) fn with_presentation(mut self, presentation: Arc<Presentation>) -> Self {
        self.presentation = Some(presentation);
        self
    }

    fn deliver_client_commands(
        &mut self,
        client: &mut ClientBridge,
        registry: &PacketRegistry,
        client_open: bool,
    ) {
        let state = client.state();
        if self.client_commands.is_empty()
            || !client_open
            || !matches!(state, ConnectionState::Config | ConnectionState::Play)
        {
            return;
        }
        while let Some(command) = self.client_commands.pop_front() {
            if let Err(e) = self.deliver_client_command(client, registry, state, command) {
                tracing::warn!("failed to deliver a player command: {e}");
            }
        }
    }

    fn deliver_client_command(
        &self,
        client: &mut ClientBridge,
        registry: &PacketRegistry,
        state: ConnectionState,
        command: ClientCommand,
    ) -> Result<(), CoreError> {
        let version = client.protocol_version;
        let config = state == ConnectionState::Config;
        let frame = match command {
            ClientCommand::PushPack(pack) => {
                let legacy = version.less_than(ProtocolVersion::V1_20_3);
                let url = pack.url.clone();
                let hash = pack.hash.clone().unwrap_or_default().to_ascii_lowercase();
                let forced = pack.required;
                let prompt = pack
                    .prompt
                    .as_ref()
                    .map(|prompt| encode_text_component(prompt, version, state));
                let id = pack.id;
                let frame = match (legacy, config) {
                    (true, true) => encode_packet(
                        &CConfigResourcePack {
                            url,
                            hash,
                            forced,
                            prompt,
                        },
                        version,
                        registry,
                    ),
                    (true, false) => encode_packet(
                        &CResourcePack {
                            url,
                            hash,
                            forced,
                            prompt,
                        },
                        version,
                        registry,
                    ),
                    (false, true) => encode_packet(
                        &CConfigResourcePackPush {
                            id,
                            url,
                            hash,
                            forced,
                            prompt,
                        },
                        version,
                        registry,
                    ),
                    (false, false) => encode_packet(
                        &CResourcePackPush {
                            id,
                            url,
                            hash,
                            forced,
                            prompt,
                        },
                        version,
                        registry,
                    ),
                }?;
                if let Some(presentation) = &self.presentation {
                    presentation.pack_pushed(Some(id), true, legacy);
                }
                frame
            }
            ClientCommand::PopPack(id) if config => {
                encode_packet(&CConfigResourcePackPop { id }, version, registry)?
            }
            ClientCommand::PopPack(id) => {
                encode_packet(&CResourcePackPop { id }, version, registry)?
            }
            ClientCommand::StoreCookie { key, data } => {
                let payload = data.to_vec();
                if config {
                    encode_packet(&CConfigStoreCookie { key, payload }, version, registry)?
                } else {
                    encode_packet(&CStoreCookie { key, payload }, version, registry)?
                }
            }
            ClientCommand::RequestCookie { key, reply } => {
                let frame = if config {
                    encode_packet(
                        &CConfigCookieRequest { key: key.clone() },
                        version,
                        registry,
                    )?
                } else {
                    encode_packet(&CCookieRequest { key: key.clone() }, version, registry)?
                };
                if let Some(presentation) = &self.presentation {
                    presentation.cookie_requested(&key, Some(reply));
                }
                frame
            }
            ClientCommand::Transfer { host, port } => {
                let port = i32::from(port);
                if config {
                    encode_packet(&CConfigTransfer { host, port }, version, registry)?
                } else {
                    encode_packet(&CTransfer { host, port }, version, registry)?
                }
            }
        };
        client.queue_frame(&frame)
    }

    pub(crate) fn deliver_messages(
        &mut self,
        client: &mut ClientBridge,
        mut backend: Option<&mut BackendBridge>,
        registry: &PacketRegistry,
        client_open: bool,
    ) {
        self.deliver_client_commands(client, registry, client_open);
        if self.messages.is_empty() {
            return;
        }
        let version = client.protocol_version;
        let ids = MessageIds::resolve(registry, version);
        let api_version = ApiVersion::new(version.0);
        let mut kept = VecDeque::new();
        while let Some(message) = self.messages.pop_front() {
            let channel = message.channel.wire_name(api_version);
            let delivered = match message.target {
                MessageTarget::Client if client_open => ids.clientbound(client.state()).map(|id| {
                    client.queue_frame(&channels::build(id, channel, &message.data, version))
                }),
                MessageTarget::Client => None,
                MessageTarget::Backend => match backend.as_deref_mut() {
                    Some(backend)
                        if matches!(
                            backend.state,
                            ConnectionState::Config | ConnectionState::Play
                        ) =>
                    {
                        ids.serverbound(backend.state).map(|id| {
                            backend.queue_frame(&channels::build(
                                id,
                                channel,
                                &message.data,
                                version,
                            ))
                        })
                    }
                    Some(_) => None,
                    None => {
                        tracing::debug!(%channel, "dropping a plugin message for a backend: the player is not on one");
                        continue;
                    }
                },
            };
            match delivered {
                Some(Ok(())) => {}
                Some(Err(e)) => tracing::warn!(%channel, "failed to deliver a plugin message: {e}"),
                None => kept.push_back(message),
            }
        }
        self.messages = kept;
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
        let command = match command {
            PlayerCommand::Kick(reason) => return CommandOutcome::Kick(reason),
            PlayerCommand::PluginMessage(message) => {
                self.messages.push_back(message);
                return CommandOutcome::Continue;
            }
            PlayerCommand::Client(command) => {
                self.client_commands.push_back(command);
                return CommandOutcome::Continue;
            }
            command => command,
        };
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
                CommandOutcome::Switch(..) => {}
                CommandOutcome::Continue => return None,
            }
        }
    }

    fn release(&mut self, client: &mut ClientBridge, registry: &PacketRegistry) -> CommandOutcome {
        while let Some(command) = self.deferred.pop_front() {
            match command {
                PlayerCommand::Kick(reason) => return CommandOutcome::Kick(reason),
                PlayerCommand::SwitchServer(target, cause) => {
                    return CommandOutcome::Switch(target, cause);
                }
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
        PlayerCommand::HeaderFooter(texts) => client.queue_frame(&packets::build_header_footer(
            &texts.0, &texts.1, version, registry,
        )?),
        PlayerCommand::ClearTitle { reset } => {
            match packets::build_clear_title(reset, version, registry)? {
                Some(frame) => client.queue_frame(&frame),
                None => Ok(()),
            }
        }
        PlayerCommand::BossBar(id, command) => {
            let packet = match command {
                BossBarCommand::Show(bar) => Some(packets::boss_bar_added(id, &bar, version)),
                BossBarCommand::Update(update) => packets::boss_bar_updated(id, &update, version),
                BossBarCommand::Hide => Some(packets::boss_bar_removed(id)),
            };
            match packet {
                Some(packet) => client.queue_frame(&encode_packet(&packet, version, registry)?),
                None => Ok(()),
            }
        }
        PlayerCommand::Kick(_)
        | PlayerCommand::SwitchServer(..)
        | PlayerCommand::PluginMessage(_)
        | PlayerCommand::Client(_) => Ok(()),
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
        tx.try_send(PlayerCommand::SwitchServer(
            ServerId::new("b"),
            ConnectCause::Switch,
        ))
        .unwrap();
        tx.try_send(message("second")).unwrap();

        assert!(matches!(
            inbox.drain(&mut client, &registry, false),
            CommandOutcome::Continue
        ));
        assert_eq!(inbox.deferred.len(), 3);

        match inbox.drain(&mut client, &registry, true) {
            CommandOutcome::Switch(target, cause) => {
                assert_eq!(target, ServerId::new("b"));
                assert_eq!(cause, ConnectCause::Switch);
            }
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
        tx.try_send(PlayerCommand::SwitchServer(
            ServerId::new("b"),
            ConnectCause::Switch,
        ))
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
