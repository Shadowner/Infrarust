use std::sync::Arc;

use bytes::Bytes;
use infrarust_api::command::CommandSource;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::command::{CommandExecuteEvent, CommandExecuteResult};
use infrarust_api::player::Player;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;

use crate::error::CoreError;
use crate::player::packets::build_system_chat_message;
use crate::services::ProxyServices;
use crate::services::command_manager::DispatchOutcome;
use crate::session::backend_bridge::BackendBridge;
use crate::session::chat_utils::{ChatIds, ChatInput, CommandInput, Outgoing, PlayerInput};
use crate::session::client_bridge::ClientBridge;

pub(crate) struct ChatScope<'a> {
    pub(crate) services: &'a ProxyServices,
    pub(crate) registry: &'a PacketRegistry,
    pub(crate) ids: &'a ChatIds,
    pub(crate) player_id: PlayerId,
    pub(crate) server: &'a ServerId,
    pub(crate) version: ProtocolVersion,
}

pub(crate) async fn intercept(
    input: PlayerInput,
    frame: PacketFrame,
    scope: &ChatScope<'_>,
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
) -> Result<Option<PacketFrame>, CoreError> {
    let Some(player) = scope
        .services
        .player_registry
        .get_player_by_id(scope.player_id)
    else {
        return Ok(Some(frame));
    };
    match input {
        PlayerInput::Chat(chat) => on_chat(chat, frame, player, scope, client, backend).await,
        PlayerInput::Command(command) => {
            on_command(command, frame, player, scope, client, backend).await
        }
    }
}

async fn on_chat(
    chat: ChatInput,
    frame: PacketFrame,
    player: Arc<dyn Player>,
    scope: &ChatScope<'_>,
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
) -> Result<Option<PacketFrame>, CoreError> {
    let event = ChatMessageEvent::new(
        player,
        chat.message().to_string(),
        chat.signed(),
        Some(scope.server.clone()),
    );
    let event = scope.services.event_bus.fire(event).await;
    match event.result() {
        ChatMessageResult::Deny { reason } => {
            acknowledge(chat.acknowledgement(scope.version), scope, backend)?;
            tell(reason.as_ref(), scope, client)?;
            Ok(None)
        }
        ChatMessageResult::Modify { message } if message != chat.message() => {
            let rewritten = encode(
                &chat.rewrite(message.clone()),
                scope.ids.message,
                scope.version,
            );
            if rewritten.is_none() {
                acknowledge(chat.acknowledgement(scope.version), scope, backend)?;
            }
            Ok(rewritten)
        }
        _ => Ok(Some(frame)),
    }
}

async fn on_command(
    command: CommandInput,
    frame: PacketFrame,
    player: Arc<dyn Player>,
    scope: &ChatScope<'_>,
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
) -> Result<Option<PacketFrame>, CoreError> {
    let event = CommandExecuteEvent::new(
        Arc::clone(&player),
        command.command().to_string(),
        command.signed(),
        Some(scope.server.clone()),
    );
    let event = scope.services.event_bus.fire(event).await;
    let acknowledgement = command.acknowledgement(scope.version);
    match event.result() {
        CommandExecuteResult::Deny { reason } => {
            acknowledge(acknowledgement, scope, backend)?;
            tell(reason.as_ref(), scope, client)?;
            Ok(None)
        }
        CommandExecuteResult::ForwardToBackend => Ok(Some(frame)),
        CommandExecuteResult::Modify { command: modified } => {
            if dispatch(player, modified, scope).await {
                acknowledge(acknowledgement, scope, backend)?;
                return Ok(None);
            }
            if modified == command.command() {
                return Ok(Some(frame));
            }
            let (outgoing, trailing) = command.rewrite(modified, scope.version);
            let rewritten = match outgoing {
                Outgoing::Message(packet) => encode(&packet, scope.ids.message, scope.version),
                Outgoing::Command(packet) => encode(&packet, scope.ids.command, scope.version),
            };
            match rewritten {
                Some(frame) => {
                    acknowledge(trailing, scope, backend)?;
                    Ok(Some(frame))
                }
                None => {
                    acknowledge(acknowledgement, scope, backend)?;
                    Ok(None)
                }
            }
        }
        _ => {
            if dispatch(player, command.command(), scope).await {
                acknowledge(acknowledgement, scope, backend)?;
                return Ok(None);
            }
            Ok(Some(frame))
        }
    }
}

async fn dispatch(player: Arc<dyn Player>, input: &str, scope: &ChatScope<'_>) -> bool {
    scope
        .services
        .command_manager
        .dispatch(CommandSource::Player(player), input)
        .await
        != DispatchOutcome::Unknown
}

fn acknowledge(
    acknowledgement: Option<impl Packet>,
    scope: &ChatScope<'_>,
    backend: &mut BackendBridge,
) -> Result<(), CoreError> {
    if let Some(frame) =
        acknowledgement.and_then(|packet| encode(&packet, scope.ids.acknowledgement, scope.version))
    {
        backend.queue_frame(&frame)?;
    }
    Ok(())
}

fn tell(
    reason: Option<&Component>,
    scope: &ChatScope<'_>,
    client: &mut ClientBridge,
) -> Result<(), CoreError> {
    let Some(reason) = reason else {
        return Ok(());
    };
    match build_system_chat_message(reason, scope.version, scope.registry) {
        Ok(frame) => client.queue_frame(&frame),
        Err(e) => {
            tracing::warn!("failed to encode a chat denial reason: {e}");
            Ok(())
        }
    }
}

fn encode(packet: &impl Packet, id: Option<i32>, version: ProtocolVersion) -> Option<PacketFrame> {
    let Some(id) = id else {
        tracing::warn!("no packet id for a rewritten chat packet in {version}");
        return None;
    };
    let mut payload = Vec::new();
    match packet.encode(&mut payload, version) {
        Ok(()) => Some(PacketFrame::new(id, Bytes::from(payload))),
        Err(e) => {
            tracing::warn!("failed to encode a rewritten chat packet: {e}");
            None
        }
    }
}
