//! Bidirectional packet forwarding loop between client and backend.
//!
//! This is the core of intercepted proxy modes. It reads packets from
//! both sides concurrently via `tokio::select!`, intercepts special
//! packets (`SetCompression`, `LoginSuccess`, Disconnect, `FinishConfig`),
//! and forwards everything else opaquely.
//!
//! Codec filters are applied to every packet BEFORE the EventBus.

use infrarust_api::command::CommandSource;
use infrarust_api::event::bus::EventBus;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::types::{Component, PlayerId, RawPacket, ServerId};
use tokio_util::sync::CancellationToken;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{
    CConfigDisconnect, CFinishConfig, SAcknowledgeFinishConfig,
};
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged,
};
use infrarust_protocol::packets::play::chat_session::SChatSessionUpdate;
use infrarust_protocol::packets::play::commands::CCommands;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::packets::play::join_game::CJoinGame;
use infrarust_protocol::packets::play::tab_complete::{
    CTabCompleteResponse, STabCompleteRequest, TabCompleteMatch,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::event_bus::conversion::{protocol_direction_to_api, protocol_state_to_api};
use crate::filter::codec_chain::{CodecFilterChain, FilterResult};
use crate::player::PlayerCommand;
use crate::player::commands::{CommandInbox, CommandOutcome};
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::kick::BackendKick;
use crate::session::server_join::ServerJoin;
use crate::util::text::encode_text_component;

/// Result of the proxy loop, determining what happens after the loop ends.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProxyLoopOutcome {
    /// Client closed its connection — full cleanup.
    ClientDisconnected,
    /// Backend closed its connection.
    /// In Phase 2A: cleanup. In Phase 4+: server switch / limbo.
    BackendDisconnected {
        reason: Option<String>,
    },
    /// Global proxy shutdown.
    Shutdown,
    /// I/O or protocol error.
    Error(CoreError),
    /// Server switch requested by plugin/command — handler should perform the switch.
    SwitchRequested {
        target: ServerId,
    },
    Kicked {
        reason: Component,
    },
    BackendKick(Box<BackendKick>),
    BackendClosed {
        reason: Option<Component>,
    },
}

/// Action to take after processing a backend → client packet.
#[derive(Debug)]
#[non_exhaustive]
enum BackendAction {
    /// Continue the loop normally.
    Continue,
    Kicked(Box<BackendKick>),
}

use super::chat_intercept::{ChatScope, intercept};
use super::chat_utils::{ChatIds, decode_player_input};

#[inline]
fn frame_to_raw(frame: &PacketFrame) -> RawPacket {
    RawPacket::new(frame.id, frame.payload.clone())
}

#[inline]
fn raw_to_frame(raw: &RawPacket) -> PacketFrame {
    PacketFrame::new(raw.packet_id, raw.data.clone())
}

const fn assert_immutable_payload(_: &bytes::Bytes) {}

#[inline]
fn filter_modified(frame: &PacketFrame, raw: &RawPacket) -> bool {
    assert_immutable_payload(&raw.data);
    assert_immutable_payload(&frame.payload);

    raw.packet_id != frame.id
        || raw.data.len() != frame.payload.len()
        || raw.data.as_ptr() != frame.payload.as_ptr()
}

struct HotIds {
    s_chat_session: Option<i32>,
    s_tab_request: Option<i32>,
    c_tab_response: Option<i32>,
    chat: ChatIds,
    c_disconnect: Option<i32>,
    c_commands: Option<i32>,
    c_join_game: Option<i32>,
    c_login_success: Option<i32>,
}

impl HotIds {
    fn resolve(registry: &PacketRegistry, version: ProtocolVersion) -> Self {
        Self {
            s_chat_session: registry.get_packet_id::<SChatSessionUpdate>(version),
            s_tab_request: registry.get_packet_id::<STabCompleteRequest>(version),
            c_tab_response: registry.get_packet_id::<CTabCompleteResponse>(version),
            chat: ChatIds::resolve(registry, version),
            c_disconnect: registry.get_packet_id::<CDisconnect>(version),
            c_commands: registry.get_packet_id::<CCommands>(version),
            c_join_game: registry.get_packet_id::<CJoinGame>(version),
            c_login_success: registry.get_packet_id::<CLoginSuccess>(version),
        }
    }

    fn milestone(
        &self,
        frame: &PacketFrame,
        backend: &BackendBridge,
        awaiting_join: bool,
    ) -> Option<Milestone> {
        if !awaiting_join {
            return None;
        }
        match backend.state {
            ConnectionState::Play if Some(frame.id) == self.c_join_game => Some(Milestone::Joined),
            ConnectionState::Login if Some(frame.id) == self.c_login_success => {
                Some(Milestone::LoggedIn)
            }
            _ => None,
        }
    }

    fn joins_game(&self, frame: &PacketFrame, backend: &BackendBridge, in_game: bool) -> bool {
        !in_game && backend.state == ConnectionState::Play && Some(frame.id) == self.c_join_game
    }
}

#[derive(Debug, Clone, Copy)]
enum Milestone {
    LoggedIn,
    Joined,
}

async fn reach(milestone: Milestone, join: &mut Option<ServerJoin>, services: &ProxyServices) {
    match milestone {
        Milestone::LoggedIn => {
            if let Some(join) = join.as_mut() {
                join.connected(&services.event_bus).await;
            }
        }
        Milestone::Joined => {
            if let Some(join) = join.take() {
                join.joined(&services.event_bus).await;
            }
        }
    }
}

/// Runs the bidirectional proxy loop between client and backend.
///
/// Both directions run concurrently via `tokio::select!`.
/// Special packets are intercepted for state management:
/// - `SetCompression`: activates compression on both bridges
/// - `LoginSuccess`: transitions Login → Config (1.20.2+) or Play
/// - `FinishConfig` / `AcknowledgeFinishConfig`: transitions Config → Play
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
    commands: &mut CommandInbox,
    services: &ProxyServices,
    player_id: PlayerId,
    server: &ServerId,
    client_codec_chain: &mut CodecFilterChain,
    server_codec_chain: &mut CodecFilterChain,
    join: &mut Option<ServerJoin>,
) -> ProxyLoopOutcome {
    let hot_ids = HotIds::resolve(registry, client.protocol_version);
    let mut tree_updates = services.command_manager.subscribe();
    let mut backend_tree: Option<CCommands> = None;
    let mut in_game = client.state() == ConnectionState::Play;
    if in_game {
        let outcome = commands.drain(client, registry, in_game);
        if let Err(e) = client.flush().await {
            return ProxyLoopOutcome::Error(e);
        }
        if let Some(end) = settle_commands(client, registry, outcome).await {
            return end;
        }
    }
    loop {
        let event = tokio::select! {
            biased;
            Some(command) = commands.recv() => LoopEvent::Command(command),
            () = shutdown.cancelled() => LoopEvent::Shutdown,
            Ok(()) = tree_updates.changed() => LoopEvent::CommandsChanged,
            event = next_frame(client, backend) => event,
        };
        match event {
            LoopEvent::Command(command) => {
                let outcome = match commands.apply(command, client, registry, in_game) {
                    CommandOutcome::Continue => commands.drain(client, registry, in_game),
                    outcome => outcome,
                };
                if let Err(e) = client.flush().await {
                    tracing::warn!("failed to flush player command: {e}");
                }
                if let Some(end) = settle_commands(client, registry, outcome).await {
                    break end;
                }
            }
            LoopEvent::CommandsChanged => {
                if let Some(tree) = backend_tree.as_ref()
                    && client.state() == ConnectionState::Play
                    && let Some(frame) = command_tree_frame(
                        tree,
                        services,
                        player_id,
                        &hot_ids,
                        client.protocol_version,
                    )
                {
                    if let Err(e) = client.queue_frame(&frame) {
                        tracing::warn!("failed to queue the refreshed command tree: {e}");
                    }
                    if let Err(e) = client.flush().await {
                        tracing::warn!("failed to flush the refreshed command tree: {e}");
                    }
                }
            }
            LoopEvent::Shutdown => {
                if let Some(reason) = commands.take_kick(client, registry, in_game) {
                    break kick(client, &reason, registry).await;
                }
                let _ = client.flush().await;
                break ProxyLoopOutcome::Shutdown;
            }
            LoopEvent::Client(frame) => match frame {
                Ok(Some(frame)) => {
                    let mut result = handle_client_to_backend(
                        client,
                        backend,
                        frame,
                        registry,
                        services,
                        player_id,
                        server,
                        client_codec_chain,
                        &hot_ids,
                    )
                    .await;
                    let mut command_outcome = commands.drain(client, registry, in_game);
                    while result.is_ok() && matches!(command_outcome, CommandOutcome::Continue) {
                        match client.try_next_frame() {
                            Ok(Some(frame)) => {
                                result = handle_client_to_backend(
                                    client,
                                    backend,
                                    frame,
                                    registry,
                                    services,
                                    player_id,
                                    server,
                                    client_codec_chain,
                                    &hot_ids,
                                )
                                .await;
                                command_outcome = commands.drain(client, registry, in_game);
                            }
                            Ok(None) => break,
                            Err(e) => result = Err(e),
                        }
                    }
                    let mut backend_lost = false;
                    if result.is_ok() {
                        result = backend.flush().await;
                        backend_lost = result.is_err();
                    }
                    if result.is_ok() {
                        result = client.flush().await;
                    }
                    if let Err(e) = result {
                        let _ = backend.flush().await;
                        let _ = client.flush().await;
                        if !e.is_expected_disconnect() {
                            break ProxyLoopOutcome::Error(e);
                        }
                        if backend_lost {
                            break ProxyLoopOutcome::BackendDisconnected {
                                reason: Some(e.to_string()),
                            };
                        }
                        break ProxyLoopOutcome::ClientDisconnected;
                    }
                    if let Some(end) = settle_commands(client, registry, command_outcome).await {
                        break end;
                    }
                }
                Ok(None) => break ProxyLoopOutcome::ClientDisconnected,
                Err(e) => break ProxyLoopOutcome::Error(e),
            },
            LoopEvent::Backend(frame) => match frame {
                Ok(Some(frame)) => {
                    let joins = hot_ids.joins_game(&frame, backend, in_game);
                    let mut milestone = hot_ids.milestone(&frame, backend, join.is_some());
                    let mut result = handle_backend_to_client(
                        client,
                        backend,
                        frame,
                        registry,
                        services,
                        player_id,
                        server_codec_chain,
                        &hot_ids,
                        &mut backend_tree,
                    )
                    .await;
                    in_game |= joins && result.is_ok();
                    let mut command_outcome = commands.drain(client, registry, in_game);
                    while milestone.is_none()
                        && matches!(result, Ok(BackendAction::Continue))
                        && matches!(command_outcome, CommandOutcome::Continue)
                    {
                        match backend.try_next_frame() {
                            Ok(Some(frame)) => {
                                let joins = hot_ids.joins_game(&frame, backend, in_game);
                                milestone = hot_ids.milestone(&frame, backend, join.is_some());
                                result = handle_backend_to_client(
                                    client,
                                    backend,
                                    frame,
                                    registry,
                                    services,
                                    player_id,
                                    server_codec_chain,
                                    &hot_ids,
                                    &mut backend_tree,
                                )
                                .await;
                                in_game |= joins && result.is_ok();
                                command_outcome = commands.drain(client, registry, in_game);
                            }
                            Ok(None) => break,
                            Err(e) => result = Err(e),
                        }
                    }
                    match result {
                        Ok(action) => {
                            if let Err(e) = client.flush().await {
                                break ProxyLoopOutcome::Error(e);
                            }
                            if let Err(e) = backend.flush().await {
                                break ProxyLoopOutcome::Error(e);
                            }
                            if let Some(milestone) = milestone {
                                reach(milestone, join, services).await;
                            }
                            match action {
                                BackendAction::Continue => {}
                                BackendAction::Kicked(kick) => {
                                    break ProxyLoopOutcome::BackendKick(kick);
                                }
                            }
                            if let Some(end) =
                                settle_commands(client, registry, command_outcome).await
                            {
                                break end;
                            }
                        }
                        Err(e) => {
                            let _ = client.flush().await;
                            break ProxyLoopOutcome::Error(e);
                        }
                    }
                }
                Ok(None) => break ProxyLoopOutcome::BackendDisconnected { reason: None },
                Err(e) if e.is_expected_disconnect() => {
                    let _ = client.flush().await;
                    break ProxyLoopOutcome::BackendDisconnected {
                        reason: Some(e.to_string()),
                    };
                }
                Err(e) => break ProxyLoopOutcome::Error(e),
            },
        }
    }
}

enum LoopEvent {
    Command(PlayerCommand),
    Shutdown,
    CommandsChanged,
    Client(Result<Option<PacketFrame>, CoreError>),
    Backend(Result<Option<PacketFrame>, CoreError>),
}

async fn next_frame(client: &mut ClientBridge, backend: &mut BackendBridge) -> LoopEvent {
    tokio::select! {
        frame = client.read_frame() => LoopEvent::Client(frame),
        frame = backend.read_frame() => LoopEvent::Backend(frame),
    }
}

async fn settle_commands(
    client: &mut ClientBridge,
    registry: &PacketRegistry,
    outcome: CommandOutcome,
) -> Option<ProxyLoopOutcome> {
    match outcome {
        CommandOutcome::Continue => None,
        CommandOutcome::Kick(reason) => Some(kick(client, &reason, registry).await),
        CommandOutcome::Switch(target) => Some(ProxyLoopOutcome::SwitchRequested { target }),
    }
}

async fn kick(
    client: &mut ClientBridge,
    reason: &Component,
    registry: &PacketRegistry,
) -> ProxyLoopOutcome {
    if let Err(e) = client.disconnect(reason, registry).await {
        tracing::debug!("failed to send the kick reason: {e}");
    }
    ProxyLoopOutcome::Kicked {
        reason: reason.clone(),
    }
}

fn command_tree_frame(
    tree: &CCommands,
    services: &ProxyServices,
    player_id: PlayerId,
    hot_ids: &HotIds,
    version: ProtocolVersion,
) -> Option<PacketFrame> {
    let id = hot_ids.c_commands?;
    let player = services.player_registry.get_player_by_id(player_id);
    let (proxy_tree, visible) = match player {
        Some(player) => {
            let visible = services
                .permission_service
                .visible_subcommands(player.permission_level());
            (
                services
                    .command_manager
                    .tree_for(Some(&CommandSource::Player(player))),
                visible,
            )
        }
        None => (
            services.command_manager.tree_for(None),
            std::collections::HashSet::new(),
        ),
    };
    let mut modified = tree.clone();
    crate::commands::brigadier::inject_proxy_commands(
        &mut modified,
        version,
        &proxy_tree,
        Some(&visible),
    );
    let mut buf = Vec::new();
    match infrarust_protocol::packets::Packet::encode(&modified, &mut buf, version) {
        Ok(()) => Some(PacketFrame::new(id, buf.into())),
        Err(e) => {
            tracing::warn!("failed to re-encode CCommands: {e}");
            None
        }
    }
}

/// Queues injected frames from a codec filter's FrameOutput.
fn send_injected_frames(
    writer: &mut impl FrameWriter,
    output: &mut infrarust_api::filter::FrameOutput,
    send_before: bool,
    send_after: bool,
) -> Result<(), CoreError> {
    if send_before {
        for raw in output.take_before() {
            writer.queue_frame(&raw_to_frame(&raw))?;
        }
    }
    if send_after {
        for raw in output.take_after() {
            writer.queue_frame(&raw_to_frame(&raw))?;
        }
    }
    Ok(())
}

/// Helper trait to abstract over client/backend bridge for queueing writes.
/// Queued frames are sent by the arm-end `flush()` in `proxy_loop`.
trait FrameWriter {
    fn queue_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError>;
}

impl FrameWriter for ClientBridge {
    fn queue_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        ClientBridge::queue_frame(self, frame)
    }
}

impl FrameWriter for BackendBridge {
    fn queue_frame(&mut self, frame: &PacketFrame) -> Result<(), CoreError> {
        BackendBridge::queue_frame(self, frame)
    }
}

/// Applies codec filter chain to a frame and handles the result.
///
/// Returns `Ok(true)` if the frame was consumed (dropped/replaced/queued with
/// injections) and should NOT be forwarded further. Returns `Ok(false)` if
/// processing should continue with the (possibly modified) frame.
fn apply_codec_filter(
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
            if filter_modified(frame, &raw) {
                *frame = raw_to_frame(&raw);
            }
            Ok(false)
        }
        FilterResult::Dropped => Ok(true),
        FilterResult::Replaced(mut output) => {
            send_injected_frames(writer, &mut output, true, true)?;
            Ok(true) // Original frame is NOT sent
        }
        FilterResult::PassWithInjections(mut output) => {
            // Queue before-injections, then the (possibly modified) original, then after-injections
            send_injected_frames(writer, &mut output, true, false)?;
            if filter_modified(frame, &raw) {
                *frame = raw_to_frame(&raw);
            }
            writer.queue_frame(frame)?;
            send_injected_frames(writer, &mut output, false, true)?;
            Ok(true) // Frame already queued with injections
        }
    }
}

/// Handles a packet from the client, forwarding it to the backend.
///
/// Order: CodecFilter → Chat/Command interception → EventBus → forward.
#[allow(clippy::too_many_arguments)]
async fn handle_client_to_backend(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
    server: &ServerId,
    codec_chain: &mut CodecFilterChain,
    hot_ids: &HotIds,
) -> Result<(), CoreError> {
    let version = client.protocol_version;
    let state = client.state();

    // In Play state: CodecFilter → chat/command → RawPacketEvent → forward
    if state == ConnectionState::Play {
        if apply_codec_filter(codec_chain, &mut frame, backend)? {
            return Ok(()); // Frame consumed by filter
        }

        // Drop SChatSessionUpdate (offline backends can't validate signatures)
        if Some(frame.id) == hot_ids.s_chat_session {
            tracing::debug!("dropping Chat Session Update (offline backend)");
            return Ok(());
        }

        if Some(frame.id) == hot_ids.s_tab_request
            && let Some(resp_id) = hot_ids.c_tab_response
            && let Ok(DecodedPacket::Typed { id: _, packet }) =
                registry.decode_frame(&frame, state, Direction::Serverbound, version)
            && let Some(req) = packet.as_any().downcast_ref::<STabCompleteRequest>()
            && let Some(input) = req.text.trim_start().strip_prefix('/')
            && let Some(player) = services.player_registry.get_player_by_id(player_id)
            && let Some(suggestions) = services
                .command_manager
                .suggest(CommandSource::Player(player), input)
                .await
        {
            let text = req.text.as_str();
            let start = text.rfind(' ').map_or(0, |i| i + 1);
            let response = CTabCompleteResponse {
                transaction_id: req.transaction_id,
                start: i32::try_from(start).unwrap_or(i32::MAX),
                length: i32::try_from(text.len() - start).unwrap_or(i32::MAX),
                matches: suggestions
                    .into_iter()
                    .map(|suggestion| TabCompleteMatch {
                        text: suggestion.text,
                        tooltip: suggestion.tooltip.map(|tooltip| {
                            encode_text_component(&tooltip, version, ConnectionState::Play)
                        }),
                    })
                    .collect(),
            };
            let mut buf = Vec::new();
            match infrarust_protocol::packets::Packet::encode(&response, &mut buf, version) {
                Ok(()) => {
                    client.queue_frame(&PacketFrame::new(resp_id, buf.into()))?;
                    return Ok(());
                }
                Err(e) => tracing::warn!("failed to encode proxy command suggestions: {e}"),
            }
        }

        if let Some(input) = decode_player_input(&frame, &hot_ids.chat, version) {
            let scope = ChatScope {
                services,
                registry,
                ids: &hot_ids.chat,
                player_id,
                server,
                version,
            };
            match intercept(input, frame, &scope, client, backend).await? {
                Some(next) => frame = next,
                None => return Ok(()),
            }
        }

        // RawPacketEvent — only fire if someone is listening for this specific packet
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(Direction::Serverbound);
        if services
            .event_bus
            .has_packet_listeners(frame.id, api_state, api_direction)
        {
            let raw_packet = RawPacket::new(frame.id, frame.payload.clone());
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
                    frame = PacketFrame::new(packet.packet_id, packet.data.clone());
                }
                infrarust_api::events::packet::RawPacketResult::Drop => {
                    return Ok(());
                }
                _ => {} // non-exhaustive
            }
        }

        backend.queue_frame(&frame)?;
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
                backend.queue_frame(&frame)?;
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
                backend.queue_frame(&frame)?;
                client.set_state(ConnectionState::Play);
                backend.set_state(ConnectionState::Play);
                codec_chain.notify_state_change(protocol_state_to_api(ConnectionState::Play));
                tracing::debug!("state transition: Config → Play (AcknowledgeFinishConfig)");
                return Ok(());
            }

            // All other typed packets: forward
            backend.queue_frame(&frame)?;
        }
        Ok(DecodedPacket::Opaque { .. }) | Err(_) => {
            // Unknown or decode error: forward opaquely
            backend.queue_frame(&frame)?;
        }
    }

    Ok(())
}

/// Handles a packet from the backend, forwarding it to the client.
///
/// Order: CodecFilter → EventBus → state interception → forward.
#[allow(clippy::too_many_arguments)]
async fn handle_backend_to_client(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    mut frame: PacketFrame,
    registry: &PacketRegistry,
    services: &ProxyServices,
    player_id: PlayerId,
    codec_chain: &mut CodecFilterChain,
    hot_ids: &HotIds,
    backend_tree: &mut Option<CCommands>,
) -> Result<BackendAction, CoreError> {
    let version = client.protocol_version;
    let state = backend.state;

    // In Play state: CodecFilter → RawPacketEvent → disconnect detection
    if state == ConnectionState::Play {
        if apply_codec_filter(codec_chain, &mut frame, client)? {
            return Ok(BackendAction::Continue); // Frame consumed by filter
        }

        // RawPacketEvent — only fire if someone is listening
        let api_state = protocol_state_to_api(state);
        let api_direction = protocol_direction_to_api(Direction::Clientbound);
        if services
            .event_bus
            .has_packet_listeners(frame.id, api_state, api_direction)
        {
            let raw_packet = RawPacket::new(frame.id, frame.payload.clone());
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
                    frame = PacketFrame::new(packet.packet_id, packet.data.clone());
                }
                infrarust_api::events::packet::RawPacketResult::Drop => {
                    return Ok(BackendAction::Continue);
                }
                _ => {} // non-exhaustive
            }
        }

        if Some(frame.id) == hot_ids.c_disconnect {
            return Ok(BackendAction::Kicked(Box::new(BackendKick::new(
                frame,
                ConnectionState::Play,
                version,
            ))));
        }
        let intercepted =
            Some(frame.id) == hot_ids.c_commands && services.config.announce_proxy_commands;
        if !intercepted {
            client.queue_frame(&frame)?;
            return Ok(BackendAction::Continue);
        }

        let tree = match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
            Ok(DecodedPacket::Typed { packet, .. }) => {
                packet.as_any().downcast_ref::<CCommands>().cloned()
            }
            Ok(DecodedPacket::Opaque { .. }) | Err(_) => None,
        };
        let injected = tree
            .as_ref()
            .and_then(|tree| command_tree_frame(tree, services, player_id, hot_ids, version));
        client.queue_frame(injected.as_ref().unwrap_or(&frame))?;
        if tree.is_some() {
            *backend_tree = tree;
        }
        return Ok(BackendAction::Continue);
    }

    // Login/Config: full interception logic
    match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
        Ok(DecodedPacket::Typed { packet, .. }) => {
            if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                let threshold = set_comp.threshold.0;
                backend.set_compression(threshold);
                client.queue_frame(&frame)?;
                client.set_compression(threshold);
                match client.compression_threshold() {
                    Some(effective) => {
                        codec_chain.notify_compression_change(effective);
                        tracing::debug!(threshold = effective, "compression activated");
                    }
                    None => {
                        tracing::debug!(threshold, "compression left disabled by backend");
                    }
                }
                return Ok(BackendAction::Continue);
            }

            // LoginSuccess — forward, transition state
            if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                client.queue_frame(&frame)?;
                // State transition happens when client sends LoginAcknowledged (1.20.2+)
                // or immediately for older versions
                if version.less_than(ProtocolVersion::V1_20_2) {
                    client.set_state(ConnectionState::Play);
                    backend.set_state(ConnectionState::Play);
                    codec_chain.notify_state_change(protocol_state_to_api(ConnectionState::Play));
                    tracing::debug!("state transition: Login → Play (pre-1.20.2)");
                }
                // For 1.20.2+, transition happens in handle_client_to_backend
                // when SLoginAcknowledged is received
                return Ok(BackendAction::Continue);
            }

            if packet.as_any().downcast_ref::<CLoginDisconnect>().is_some()
                || packet
                    .as_any()
                    .downcast_ref::<CConfigDisconnect>()
                    .is_some()
            {
                return Ok(BackendAction::Kicked(Box::new(BackendKick::new(
                    frame, state, version,
                ))));
            }

            // FinishConfig — forward, state transition happens when client ACKs
            if packet.as_any().downcast_ref::<CFinishConfig>().is_some() {
                services.registry_codec_cache.finalize(version);
                client.queue_frame(&frame)?;
                // Transition happens in handle_client_to_backend
                // when SAcknowledgeFinishConfig is received
                return Ok(BackendAction::Continue);
            }

            if state == ConnectionState::Config {
                let is_known_packs = registry
                    .get_packet_id::<infrarust_protocol::CKnownPacks>(version)
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
            client.queue_frame(&frame)?;
        }
        Ok(DecodedPacket::Opaque { .. }) => {
            if state == ConnectionState::Config {
                services
                    .registry_codec_cache
                    .collect_registry_frame(version, frame.clone());
            }
            client.queue_frame(&frame)?;
        }
        Err(e) => {
            tracing::warn!("failed to decode backend frame: {e}");
            // Forward anyway (best effort)
            client.queue_frame(&frame)?;
        }
    }

    Ok(BackendAction::Continue)
}
