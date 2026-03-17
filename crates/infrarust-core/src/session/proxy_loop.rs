//! Bidirectional packet forwarding loop between client and backend.
//!
//! This is the core of intercepted proxy modes. It reads packets from
//! both sides concurrently via `tokio::select!`, intercepts special
//! packets (`SetCompression`, `LoginSuccess`, Disconnect, `FinishConfig`),
//! and forwards everything else opaquely.

use infrarust_api::event::ResultedEvent;
use infrarust_api::event::bus::EventBus;
use infrarust_api::types::PlayerId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged,
};
use infrarust_protocol::packets::play::chat::{SChatCommand, SChatMessage};
use infrarust_protocol::packets::play::chat_session::SChatSessionUpdate;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::error::CoreError;
use crate::event_bus::conversion::{protocol_direction_to_api, protocol_state_to_api};
use crate::player::PlayerCommand;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;

/// Result of the proxy loop, determining what happens after the loop ends.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProxyLoopOutcome {
    /// Client closed its connection — full cleanup.
    ClientDisconnected,
    /// Backend closed its connection.
    /// In Phase 2A: cleanup. In Phase 4+: server switch / limbo.
    BackendDisconnected { reason: Option<String> },
    /// Global proxy shutdown.
    Shutdown,
    /// I/O or protocol error.
    Error(CoreError),
}

/// Action to take after processing a backend → client packet.
#[derive(Debug)]
#[non_exhaustive]
enum BackendAction {
    /// Continue the loop normally.
    Continue,
    /// Backend sent a disconnect packet.
    Disconnected(Option<String>),
}

/// What a chat/command packet resolved to.
enum ChatAction {
    /// A slash command (without the leading `/`).
    Command(String),
    /// A regular chat message.
    Message(String),
}

/// Runs the bidirectional proxy loop between client and backend.
///
/// Both directions run concurrently via `tokio::select!`.
/// Special packets are intercepted for state management:
/// - `SetCompression`: activates compression on both bridges
/// - `LoginSuccess`: transitions Login → Config (1.20.2+) or Play
/// - `FinishConfig` / `AcknowledgeFinishConfig`: transitions Config → Play
/// - `Disconnect`: forwards and terminates
///
/// In Play state, only `CDisconnect` is intercepted. All other packets
/// are forwarded opaquely for maximum performance and to avoid decode
/// errors from version-specific packet ID mismatches.
pub async fn proxy_loop(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    registry: &PacketRegistry,
    shutdown: CancellationToken,
    mut command_rx: mpsc::Receiver<PlayerCommand>,
    services: &ProxyServices,
    player_id: PlayerId,
) -> ProxyLoopOutcome {
    loop {
        tokio::select! {
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        if let Err(e) = handle_client_to_backend(client, backend, frame, registry, services, player_id).await {

                            return ProxyLoopOutcome::Error(e);
                        }
                    }
                    Ok(None) => return ProxyLoopOutcome::ClientDisconnected,
                    Err(e) => return ProxyLoopOutcome::Error(e),
                }
            }
            frame = backend.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        match handle_backend_to_client(client, backend, frame, registry, services, player_id).await {
                            Ok(BackendAction::Continue) => {}
                            Ok(BackendAction::Disconnected(reason)) => {
                                return ProxyLoopOutcome::BackendDisconnected { reason };
                            }
                            Err(e) => return ProxyLoopOutcome::Error(e),
                        }
                    }
                    Ok(None) => return ProxyLoopOutcome::BackendDisconnected { reason: None },
                    Err(e) => return ProxyLoopOutcome::Error(e),
                }
            }
            Some(cmd) = command_rx.recv() => {
                match handle_player_command(client, cmd, registry).await {
                    Ok(true) => return ProxyLoopOutcome::ClientDisconnected, // Kick
                    Ok(false) => {} // Continue
                    Err(e) => {
                        tracing::warn!("failed to handle player command: {e}");
                    }
                }
            }
            () = shutdown.cancelled() => {
                return ProxyLoopOutcome::Shutdown;
            }
        }
    }
}

/// Handles a player command from the plugin system.
///
/// Returns `Ok(true)` if the connection should be terminated (kick),
/// `Ok(false)` to continue normally.
async fn handle_player_command(
    client: &mut ClientBridge,
    cmd: PlayerCommand,
    registry: &PacketRegistry,
) -> Result<bool, CoreError> {
    use crate::player::packets;

    let version = client.protocol_version;

    match cmd {
        PlayerCommand::SendMessage(component) => {
            let frame = packets::build_system_chat_message(&component, version, registry)?;
            client.write_frame(&frame).await?;
        }
        PlayerCommand::SendActionBar(component) => {
            let frame = packets::build_action_bar(&component, version, registry)?;
            client.write_frame(&frame).await?;
        }
        PlayerCommand::SendTitle(title_data) => {
            let frames = packets::build_title_packets(&title_data, version, registry)?;
            for frame in &frames {
                client.write_frame(frame).await?;
            }
        }
        PlayerCommand::SendPacket(raw_packet) => {
            let frame = PacketFrame {
                id: raw_packet.packet_id,
                payload: raw_packet.data,
            };
            client.write_frame(&frame).await?;
        }
        PlayerCommand::Kick(reason) => {
            let frame = packets::build_disconnect(&reason, version, registry)?;
            client.write_frame(&frame).await?;
            return Ok(true);
        }
        PlayerCommand::SwitchServer(target) => {
            tracing::warn!(
                target = %target,
                "SwitchServer not implemented in Phase 0"
            );
        }
    }

    Ok(false)
}

/// Detects if a frame is a chat message or slash command.
///
/// Returns `Some(ChatAction)` if the frame matches a serverbound chat
/// packet (`SChatMessage` or `SChatCommand`), `None` otherwise.
fn detect_chat_or_command(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: infrarust_protocol::version::ProtocolVersion,
) -> Option<ChatAction> {
    // Check if it's a SChatCommand packet (1.19+)
    let chat_cmd_id = registry.get_packet_id::<SChatCommand>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    if Some(frame.id) == chat_cmd_id {
        // Decode just the command string
        let mut data = frame.payload.as_ref();
        if let Ok(decoded) = SChatCommand::decode(&mut data, version) {
            return Some(ChatAction::Command(decoded.command));
        }
    }

    // Check if it's a SChatMessage packet
    let chat_msg_id = registry.get_packet_id::<SChatMessage>(
        ConnectionState::Play,
        Direction::Serverbound,
        version,
    );
    if Some(frame.id) == chat_msg_id {
        // Decode just the message string
        let mut data = frame.payload.as_ref();
        if let Ok(decoded) = SChatMessage::decode(&mut data, version) {
            if decoded.message.starts_with('/') {
                // Pre-1.19 style: commands sent as chat messages with leading /
                return Some(ChatAction::Command(decoded.message[1..].to_string()));
            }
            return Some(ChatAction::Message(decoded.message));
        }
    }

    None
}

use infrarust_protocol::Packet;

/// Handles a packet from the client, forwarding it to the backend.
///
/// - In Login/Config state: decodes for state transition detection
///   (`SLoginAcknowledged`, `SAcknowledgeFinishConfig`).
/// - In Play state: intercepts chat/commands, fires RawPacketEvent for
///   packets with listeners, drops `SChatSessionUpdate`, and forwards
///   everything else opaquely.
async fn handle_client_to_backend(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
) -> Result<(), CoreError> {
    let version = client.protocol_version;
    let state = client.state();

    // In Play state: chat/command interception + RawPacketEvent + opaque forward
    if state == ConnectionState::Play {
        // Drop SChatSessionUpdate (offline backends can't validate signatures)
        let chat_session_id = registry.get_packet_id::<SChatSessionUpdate>(
            ConnectionState::Play,
            Direction::Serverbound,
            version,
        );
        if Some(frame.id) == chat_session_id {
            tracing::debug!("dropping Chat Session Update (offline backend)");
            return Ok(());
        }

        // Chat/command detection (serverbound only)
        if let Some(action) = detect_chat_or_command(&frame, registry, version) {
            match action {
                ChatAction::Command(input) => {
                    // CommandManager first
                    let handled = services
                        .command_manager
                        .dispatch(
                            Some(player_id),
                            &input,
                            services.player_registry.as_ref(),
                        )
                        .await;
                    if handled {
                        return Ok(()); // Command consumed, don't forward
                    }
                    // Unknown command → forward normally to backend
                }
                ChatAction::Message(text) => {
                    // Fire ChatMessageEvent
                    let chat_event =
                        infrarust_api::events::chat::ChatMessageEvent::new(player_id, text);
                    let chat_event = services.event_bus.fire(chat_event).await;
                    match chat_event.result() {
                        infrarust_api::events::chat::ChatMessageResult::Deny { .. } => {
                            return Ok(()); // Don't forward
                        }
                        infrarust_api::events::chat::ChatMessageResult::Allow => {
                            // Forward normally below
                        }
                        infrarust_api::events::chat::ChatMessageResult::Modify { .. } => {
                            // Modifying signed messages is not possible (1.19+)
                            // Forward the original for now
                        }
                        _ => {} // non-exhaustive
                    }
                }
            }
        }

        // RawPacketEvent — only fire if someone is listening for this specific packet
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(Direction::Serverbound);
        if services
            .event_bus
            .has_packet_listeners(frame.id, api_state, api_direction)
        {
            let raw_packet = infrarust_api::types::RawPacket::new(
                frame.id,
                bytes::Bytes::copy_from_slice(&frame.payload),
            );
            let mut event = infrarust_api::events::packet::RawPacketEvent::new(
                player_id,
                api_direction,
                raw_packet,
            );
            services
                .event_bus
                .fire_packet_event(frame.id, api_state, api_direction, &mut event)
                .await;
            match event.result() {
                infrarust_api::events::packet::RawPacketResult::Pass => {}
                infrarust_api::events::packet::RawPacketResult::Modify { packet } => {
                    frame = PacketFrame {
                        id: packet.packet_id,
                        payload: packet.data.clone(),
                    };
                }
                infrarust_api::events::packet::RawPacketResult::Drop => {
                    return Ok(());
                }
                _ => {} // non-exhaustive
            }
        }

        backend.write_frame(&frame).await?;
        return Ok(());
    }

    // Login/Config: decode for state transition detection
    match registry.decode_frame(&frame, state, Direction::Serverbound, version) {
        Ok(DecodedPacket::Typed { packet, .. }) => {
            if packet
                .as_any()
                .downcast_ref::<SLoginAcknowledged>()
                .is_some()
            {
                // Client acknowledged login success → transition to Config
                backend.write_frame(&frame).await?;
                client.set_state(ConnectionState::Config);
                backend.set_state(ConnectionState::Config);
                tracing::debug!("state transition: Login → Config (LoginAcknowledged)");
                return Ok(());
            }

            if packet
                .as_any()
                .downcast_ref::<SAcknowledgeFinishConfig>()
                .is_some()
            {
                // Client acknowledged finish config → transition to Play
                backend.write_frame(&frame).await?;
                client.set_state(ConnectionState::Play);
                backend.set_state(ConnectionState::Play);
                tracing::debug!("state transition: Config → Play (AcknowledgeFinishConfig)");
                return Ok(());
            }

            // All other typed packets: forward
            backend.write_frame(&frame).await?;
        }
        Ok(DecodedPacket::Opaque { .. }) | Err(_) => {
            // Unknown or decode error: forward opaquely
            backend.write_frame(&frame).await?;
        }
    }

    Ok(())
}

/// Handles a packet from the backend, forwarding it to the client.
///
/// - In Login/Config state: intercepts `SetCompression`, `LoginSuccess`,
///   `Disconnect`, and `FinishConfig` for state management.
/// - In Play state: fires RawPacketEvent for packets with listeners,
///   only intercepts `CDisconnect` for disconnect reason logging.
///   All other packets are forwarded opaquely.
async fn handle_backend_to_client(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
) -> Result<BackendAction, CoreError> {
    let version = client.protocol_version;
    let state = backend.state;

    // In Play state: RawPacketEvent + disconnect detection
    if state == ConnectionState::Play {
        // RawPacketEvent — only fire if someone is listening
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(Direction::Clientbound);
        if services
            .event_bus
            .has_packet_listeners(frame.id, api_state, api_direction)
        {
            let raw_packet = infrarust_api::types::RawPacket::new(
                frame.id,
                bytes::Bytes::copy_from_slice(&frame.payload),
            );
            let mut event = infrarust_api::events::packet::RawPacketEvent::new(
                player_id,
                api_direction,
                raw_packet,
            );
            services
                .event_bus
                .fire_packet_event(frame.id, api_state, api_direction, &mut event)
                .await;
            match event.result() {
                infrarust_api::events::packet::RawPacketResult::Pass => {}
                infrarust_api::events::packet::RawPacketResult::Modify { packet } => {
                    frame = PacketFrame {
                        id: packet.packet_id,
                        payload: packet.data.clone(),
                    };
                }
                infrarust_api::events::packet::RawPacketResult::Drop => {
                    return Ok(BackendAction::Continue);
                }
                _ => {} // non-exhaustive
            }
        }

        // Disconnect detection
        match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
            Ok(DecodedPacket::Typed { packet, .. }) => {
                if packet.as_any().downcast_ref::<CDisconnect>().is_some() {
                    client.write_frame(&frame).await?;
                    return Ok(BackendAction::Disconnected(Some(
                        "backend disconnect".to_string(),
                    )));
                }
                // KeepAlive or other typed: forward
                client.write_frame(&frame).await?;
            }
            Ok(DecodedPacket::Opaque { .. }) => {
                client.write_frame(&frame).await?;
            }
            Err(_) => {
                // Should not happen with encode_only cleanup, but forward anyway
                client.write_frame(&frame).await?;
            }
        }
        return Ok(BackendAction::Continue);
    }

    // Login/Config: full interception logic
    match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
        Ok(DecodedPacket::Typed { packet, .. }) => {
            // SetCompression — activate on both sides, forward to client
            if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                let threshold = set_comp.threshold.0;
                backend.set_compression(threshold);
                client.set_compression(threshold);
                client.write_frame(&frame).await?;
                tracing::debug!(threshold, "compression activated");
                return Ok(BackendAction::Continue);
            }

            // LoginSuccess — forward, transition state
            if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                client.write_frame(&frame).await?;
                // State transition happens when client sends LoginAcknowledged (1.20.2+)
                // or immediately for older versions
                if version.less_than(infrarust_protocol::version::ProtocolVersion::V1_20_2) {
                    client.set_state(ConnectionState::Play);
                    backend.set_state(ConnectionState::Play);
                    tracing::debug!("state transition: Login → Play (pre-1.20.2)");
                }
                // For 1.20.2+, transition happens in handle_client_to_backend
                // when SLoginAcknowledged is received
                return Ok(BackendAction::Continue);
            }

            // LoginDisconnect
            if let Some(disconnect) = packet.as_any().downcast_ref::<CLoginDisconnect>() {
                client.write_frame(&frame).await?;
                return Ok(BackendAction::Disconnected(Some(disconnect.reason.clone())));
            }

            // Play Disconnect (should not occur in Login/Config, but handle defensively)
            if packet.as_any().downcast_ref::<CDisconnect>().is_some() {
                client.write_frame(&frame).await?;
                return Ok(BackendAction::Disconnected(Some(
                    "backend disconnect".to_string(),
                )));
            }

            // FinishConfig — forward, state transition happens when client ACKs
            if packet.as_any().downcast_ref::<CFinishConfig>().is_some() {
                client.write_frame(&frame).await?;
                // Transition happens in handle_client_to_backend
                // when SAcknowledgeFinishConfig is received
                return Ok(BackendAction::Continue);
            }

            // All other typed packets: forward
            client.write_frame(&frame).await?;
        }
        Ok(DecodedPacket::Opaque { .. }) => {
            client.write_frame(&frame).await?;
        }
        Err(e) => {
            tracing::warn!("failed to decode backend frame: {e}");
            // Forward anyway (best effort)
            client.write_frame(&frame).await?;
        }
    }

    Ok(BackendAction::Continue)
}
