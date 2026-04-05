//! Bidirectional packet forwarding loop between client and backend.
//!
//! This is the core of intercepted proxy modes. It reads packets from
//! both sides concurrently via `tokio::select!`, intercepts special
//! packets (`SetCompression`, `LoginSuccess`, Disconnect, `FinishConfig`),
//! and forwards everything else opaquely.
//!
//! Codec filters are applied to every packet BEFORE the EventBus.

use infrarust_api::event::ResultedEvent;
use infrarust_api::event::bus::EventBus;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{PlayerId, RawPacket, ServerId};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged,
};
use infrarust_protocol::packets::play::chat_session::SChatSessionUpdate;
use infrarust_protocol::packets::play::commands::CCommands;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::tab_complete::{
    CTabCompleteResponse, STabCompleteRequest, TabCompleteMatch,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::error::CoreError;
use crate::event_bus::conversion::{protocol_direction_to_api, protocol_state_to_api};
use crate::filter::codec_chain::{CodecFilterChain, FilterResult};
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
    /// Server switch requested by plugin/command — handler should perform the switch.
    SwitchRequested { target: ServerId },
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

use super::chat_utils::{ChatAction, detect_chat_or_command};

#[inline]
fn frame_to_raw(frame: &PacketFrame) -> RawPacket {
    RawPacket::new(frame.id, frame.payload.clone())
}

#[inline]
fn raw_to_frame(raw: &RawPacket) -> PacketFrame {
    PacketFrame {
        id: raw.packet_id,
        payload: raw.data.clone(),
    }
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
/// Codec filters are applied to every packet BEFORE the EventBus.
/// In Play state, only `CDisconnect` is intercepted. All other packets
/// are forwarded opaquely for maximum performance.
#[allow(clippy::too_many_arguments)]
pub async fn proxy_loop(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    registry: &PacketRegistry,
    shutdown: CancellationToken,
    command_rx: &mut mpsc::Receiver<PlayerCommand>,
    services: &ProxyServices,
    player_id: PlayerId,
    client_codec_chain: &mut CodecFilterChain,
    server_codec_chain: &mut CodecFilterChain,
) -> ProxyLoopOutcome {
    let outcome = loop {
        tokio::select! {
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        if let Err(e) = handle_client_to_backend(client, backend, frame, registry, services, player_id, client_codec_chain).await {

                            break ProxyLoopOutcome::Error(e);
                        }
                    }
                    Ok(None) => break ProxyLoopOutcome::ClientDisconnected,
                    Err(e) => break ProxyLoopOutcome::Error(e),
                }
            }
            frame = backend.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        match handle_backend_to_client(client, backend, frame, registry, services, player_id, server_codec_chain).await {
                            Ok(BackendAction::Continue) => {}
                            Ok(BackendAction::Disconnected(reason)) => {
                                break ProxyLoopOutcome::BackendDisconnected { reason };
                            }
                            Err(e) => break ProxyLoopOutcome::Error(e),
                        }
                    }
                    Ok(None) => break ProxyLoopOutcome::BackendDisconnected { reason: None },
                    Err(e) => break ProxyLoopOutcome::Error(e),
                }
            }
            Some(cmd) = command_rx.recv() => {
                match handle_player_command(client, cmd, registry).await {
                    Ok(CommandResult::Continue) => {}
                    Ok(CommandResult::Kick) => break ProxyLoopOutcome::ClientDisconnected,
                    Ok(CommandResult::Switch(target)) => {
                        break ProxyLoopOutcome::SwitchRequested { target };
                    }
                    Err(e) => {
                        tracing::warn!("failed to handle player command: {e}");
                    }
                }
            }
            () = shutdown.cancelled() => {
                break ProxyLoopOutcome::Shutdown;
            }
        }
    };

    // Cleanup filter chains
    client_codec_chain.close();
    server_codec_chain.close();

    outcome
}

/// What `handle_player_command` resolved to.
enum CommandResult {
    /// Continue the loop normally.
    Continue,
    /// Kick the player — terminate the connection.
    Kick,
    /// Switch to a different server.
    Switch(ServerId),
}

/// Handles a player command from the plugin system.
async fn handle_player_command(
    client: &mut ClientBridge,
    cmd: PlayerCommand,
    registry: &PacketRegistry,
) -> Result<CommandResult, CoreError> {
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
            let frame = raw_to_frame(&raw_packet);
            client.write_frame(&frame).await?;
        }
        PlayerCommand::Kick(reason) => {
            let frame = packets::build_disconnect(&reason, version, registry)?;
            client.write_frame(&frame).await?;
            return Ok(CommandResult::Kick);
        }
        PlayerCommand::SwitchServer(target) => {
            return Ok(CommandResult::Switch(target));
        }
    }

    Ok(CommandResult::Continue)
}

/// Sends injected frames from a codec filter's FrameOutput.
async fn send_injected_frames(
    writer: &mut impl FrameWriter,
    output: &mut infrarust_api::filter::FrameOutput,
    send_before: bool,
    send_after: bool,
) -> Result<(), CoreError> {
    if send_before {
        for raw in output.take_before() {
            writer.write_frame(&raw_to_frame(&raw)).await?;
        }
    }
    if send_after {
        for raw in output.take_after() {
            writer.write_frame(&raw_to_frame(&raw)).await?;
        }
    }
    Ok(())
}

/// Helper trait to abstract over client/backend bridge for writing.
trait FrameWriter {
    fn write_frame(
        &mut self,
        frame: &PacketFrame,
    ) -> impl std::future::Future<Output = Result<(), CoreError>> + Send;
}

impl FrameWriter for ClientBridge {
    async fn write_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        ClientBridge::write_frame(self, frame).await
    }
}

impl FrameWriter for BackendBridge {
    async fn write_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        BackendBridge::write_frame(self, frame).await
    }
}

/// Applies codec filter chain to a frame and handles the result.
///
/// Returns `Ok(true)` if the frame was consumed (dropped/replaced) and should
/// NOT be forwarded further. Returns `Ok(false)` if processing should continue
/// with the (possibly modified) frame.
async fn apply_codec_filter(
    chain: &mut CodecFilterChain,
    frame: &mut PacketFrame,
    writer: &mut impl FrameWriter,
) -> Result<bool, CoreError> {
    if chain.is_empty() {
        return Ok(false);
    }

    let mut raw = frame_to_raw(frame);
    match chain.process(&mut raw) {
        FilterResult::Pass => {
            // Update the frame in case a filter modified it in place
            *frame = raw_to_frame(&raw);
            Ok(false)
        }
        FilterResult::Dropped => Ok(true),
        FilterResult::Replaced(mut output) => {
            send_injected_frames(writer, &mut output, true, true).await?;
            Ok(true) // Original frame is NOT sent
        }
        FilterResult::PassWithInjections(mut output) => {
            // Send before-injections, then the (possibly modified) original, then after-injections
            send_injected_frames(writer, &mut output, true, false).await?;
            *frame = raw_to_frame(&raw);
            writer.write_frame(frame).await?;
            send_injected_frames(writer, &mut output, false, true).await?;
            Ok(true) // Frame already sent with injections
        }
    }
}

/// Handles a packet from the client, forwarding it to the backend.
///
/// Order: CodecFilter → Chat/Command interception → EventBus → forward.
async fn handle_client_to_backend(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
    codec_chain: &mut CodecFilterChain,
) -> Result<(), CoreError> {
    let version = client.protocol_version;
    let state = client.state();

    // In Play state: CodecFilter → chat/command → RawPacketEvent → forward
    if state == ConnectionState::Play {
        if apply_codec_filter(codec_chain, &mut frame, backend).await? {
            return Ok(()); // Frame consumed by filter
        }

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

        let tab_complete_id = registry.get_packet_id::<STabCompleteRequest>(
            ConnectionState::Play,
            Direction::Serverbound,
            version,
        );
        if Some(frame.id) == tab_complete_id
            && let Ok(DecodedPacket::Typed { id: _, packet }) =
                registry.decode_frame(&frame, state, Direction::Serverbound, version)
            && let Some(req) = packet.as_any().downcast_ref::<STabCompleteRequest>()
        {
            let text = req.text.trim_start();
            let should_intercept = if text.starts_with("/infrarust ") || text.starts_with("/ir ") {
                true
            } else {
                let cmd_name = text
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                !cmd_name.is_empty() && services.command_manager.is_plugin_command(cmd_name)
            };

            if should_intercept {
                let cmd_input = text.trim_start_matches('/');
                let suggestions = services.command_manager.tab_complete(cmd_input);
                let last_space = text.rfind(' ').unwrap_or(0) + 1;
                let response = CTabCompleteResponse {
                    transaction_id: req.transaction_id,
                    start: last_space as i32,
                    length: (text.len() - last_space) as i32,
                    matches: suggestions
                        .into_iter()
                        .map(|s| TabCompleteMatch {
                            text: s,
                            tooltip: None,
                        })
                        .collect(),
                };
                let resp_id = registry.get_packet_id::<CTabCompleteResponse>(
                    ConnectionState::Play,
                    Direction::Clientbound,
                    version,
                );
                if let Some(resp_id) = resp_id {
                    let mut buf = Vec::new();
                    if infrarust_protocol::packets::Packet::encode(&response, &mut buf, version)
                        .is_ok()
                    {
                        let resp_frame = PacketFrame {
                            id: resp_id,
                            payload: buf.into(),
                        };
                        client.write_frame(&resp_frame).await?;
                        return Ok(());
                    }
                }
            }
        }

        // Chat/command detection (serverbound only)
        if let Some(action) = detect_chat_or_command(&frame, registry, version) {
            match action {
                ChatAction::Command(input) => {
                    // CommandManager first
                    let handled = services
                        .command_manager
                        .dispatch(Some(player_id), &input, services.player_registry.as_ref())
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
            let raw_packet = infrarust_api::types::RawPacket::new(frame.id, frame.payload.clone());
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
                codec_chain.notify_state_change(protocol_state_to_api(ConnectionState::Config));
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
                codec_chain.notify_state_change(protocol_state_to_api(ConnectionState::Play));
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
/// Order: CodecFilter → EventBus → state interception → forward.
async fn handle_backend_to_client(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
    codec_chain: &mut CodecFilterChain,
) -> Result<BackendAction, CoreError> {
    let version = client.protocol_version;
    let state = backend.state;

    // In Play state: CodecFilter → RawPacketEvent → disconnect detection
    if state == ConnectionState::Play {
        if apply_codec_filter(codec_chain, &mut frame, client).await? {
            return Ok(BackendAction::Continue); // Frame consumed by filter
        }

        // RawPacketEvent — only fire if someone is listening
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(Direction::Clientbound);
        if services
            .event_bus
            .has_packet_listeners(frame.id, api_state, api_direction)
        {
            let raw_packet = infrarust_api::types::RawPacket::new(frame.id, frame.payload.clone());
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
            Ok(DecodedPacket::Typed { id, packet }) => {
                if let Some(disc) = packet.as_any().downcast_ref::<CDisconnect>() {
                    client.write_frame(&frame).await?;
                    let reason = disc
                        .as_json()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| String::from_utf8_lossy(&disc.reason).to_string());
                    return Ok(BackendAction::Disconnected(Some(reason)));
                }
                if let Some(commands) = packet.as_any().downcast_ref::<CCommands>() {
                    if services.config.announce_proxy_commands {
                        let mut modified = commands.clone();
                        let plugin_cmds = services.command_manager.list_plugin_commands();
                        let visible =
                            services
                                .player_registry
                                .get_player_by_id(player_id)
                                .map(|p| {
                                    services
                                        .permission_service
                                        .visible_subcommands(p.permission_level())
                                });
                        crate::commands::brigadier::inject_proxy_commands(
                            &mut modified,
                            version,
                            &plugin_cmds,
                            visible.as_ref(),
                        );
                        let mut buf = Vec::new();
                        if let Err(e) = infrarust_protocol::packets::Packet::encode(
                            &modified, &mut buf, version,
                        ) {
                            tracing::warn!("failed to re-encode CCommands: {e}");
                            client.write_frame(&frame).await?;
                        } else {
                            let new_frame = PacketFrame {
                                id,
                                payload: buf.into(),
                            };
                            client.write_frame(&new_frame).await?;
                        }
                    } else {
                        client.write_frame(&frame).await?;
                    }
                } else {
                    client.write_frame(&frame).await?;
                }
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
                codec_chain.notify_compression_change(threshold);
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
                    codec_chain.notify_state_change(protocol_state_to_api(ConnectionState::Play));
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
                services.registry_codec_cache.finalize(version);
                client.write_frame(&frame).await?;
                // Transition happens in handle_client_to_backend
                // when SAcknowledgeFinishConfig is received
                return Ok(BackendAction::Continue);
            }

            if state == ConnectionState::Config {
                let is_known_packs = registry
                    .get_packet_id::<infrarust_protocol::CKnownPacks>(
                        ConnectionState::Config,
                        Direction::Clientbound,
                        version,
                    )
                    .is_some_and(|id| id == frame.id);

                if is_known_packs {
                    services
                        .registry_codec_cache
                        .collect_known_packs_frame(version, frame.clone());
                } else {
                    services
                        .registry_codec_cache
                        .collect_registry_frame(version, frame.clone());
                }
            }

            // All other typed packets: forward
            client.write_frame(&frame).await?;
        }
        Ok(DecodedPacket::Opaque { .. }) => {
            if state == ConnectionState::Config {
                services
                    .registry_codec_cache
                    .collect_registry_frame(version, frame.clone());
            }
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
