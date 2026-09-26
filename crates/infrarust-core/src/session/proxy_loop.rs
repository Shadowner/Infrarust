//! Bidirectional packet forwarding loop between client and backend.
//!
//! This is the core of intercepted proxy modes. It reads packets from
//! both sides concurrently via `tokio::select!`, intercepts special
//! packets (`SetCompression`, `LoginSuccess`, Disconnect, `FinishConfig`),
//! and forwards everything else opaquely.
//!
//! Codec filters are applied to every packet BEFORE the EventBus.

use std::collections::VecDeque;
use std::sync::Arc;

use infrarust_api::command::{CommandSource, Suggestion};
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::messaging::ChannelId;
use infrarust_api::player::Player;
use infrarust_api::types::{Component, ProtocolVersion as ApiVersion, RawPacket, ServerId};
use tokio::time::Instant;

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
use infrarust_protocol::packets::play::keepalive::{CKeepAlive, SKeepAlive};
use infrarust_protocol::packets::play::start_configuration::{
    CStartConfiguration, SAcknowledgeConfiguration,
};
use infrarust_protocol::packets::play::tab_complete::{
    CTabCompleteResponse, STabCompleteRequest, TabCompleteMatch,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::event_bus::conversion::protocol_state_to_api;
use crate::filter::codec_chain::{CodecFilterChain, FilterResult};
use crate::player::commands::CommandOutcome;
use crate::player::{PlayerCommand, PlayerSession};
use crate::plugin_messaging::channels::{self, MessageIds};
use crate::services::ProxyServices;
use crate::services::command_manager::Prepared;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::BackendKick;
use crate::session::presentation;
use crate::session::server_join::ServerJoin;
use crate::util::text::encode_text_component;

/// Result of the proxy loop, determining what happens after the loop ends.
#[derive(Debug)]
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
        cause: ConnectCause,
    },
    Kicked {
        reason: Component,
    },
    BackendKick(Box<BackendKick>),
    BackendClosed {
        reason: Option<Component>,
    },
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

struct HotIds {
    s_chat_session: Option<i32>,
    s_tab_request: Option<i32>,
    c_tab_response: Option<i32>,
    chat: ChatIds,
    c_disconnect: Option<i32>,
    c_commands: Option<i32>,
    c_join_game: Option<i32>,
    c_login_success: Option<i32>,
    c_keepalive: Option<i32>,
    s_keepalive: Option<i32>,
    c_start_config: Option<i32>,
    s_ack_config: Option<i32>,
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
            c_keepalive: registry.get_packet_id::<CKeepAlive>(version),
            s_keepalive: registry.get_packet_id::<SKeepAlive>(version),
            c_start_config: registry.get_packet_id::<CStartConfiguration>(version),
            s_ack_config: registry.get_packet_id::<SAcknowledgeConfiguration>(version),
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

    fn joins_game(&self, frame: &PacketFrame, reading: ConnectionState, in_game: bool) -> bool {
        !in_game && reading == ConnectionState::Play && Some(frame.id) == self.c_join_game
    }
}

#[derive(Debug, Clone, Copy)]
enum Milestone {
    LoggedIn,
    Joined,
}

const TRACKED_KEEPALIVES: usize = 8;

struct LoopState {
    keepalives: VecDeque<(i64, Instant)>,
    config_closing: bool,
    reconfiguring: bool,
    presentation_lost: bool,
}

impl LoopState {
    const fn new() -> Self {
        Self {
            keepalives: VecDeque::new(),
            config_closing: false,
            reconfiguring: false,
            presentation_lost: false,
        }
    }

    const fn client_reading(client: &ClientBridge) -> ConnectionState {
        if client.awaits_config_ack() {
            ConnectionState::Play
        } else {
            client.state()
        }
    }

    const fn backend_reading(&self, backend: &BackendBridge) -> ConnectionState {
        if self.reconfiguring {
            ConnectionState::Config
        } else {
            backend.state
        }
    }

    fn keepalive_sent(&mut self, frame: &PacketFrame, version: ProtocolVersion) {
        use infrarust_protocol::packets::Packet;
        let Ok(keepalive) = CKeepAlive::decode(&mut frame.payload.as_ref(), version) else {
            return;
        };
        if self.keepalives.len() == TRACKED_KEEPALIVES {
            self.keepalives.pop_front();
        }
        self.keepalives.push_back((keepalive.id, Instant::now()));
    }

    fn keepalive_answered(
        &mut self,
        session: &PlayerSession,
        frame: &PacketFrame,
        version: ProtocolVersion,
    ) {
        use infrarust_protocol::packets::Packet;
        let Ok(keepalive) = SKeepAlive::decode(&mut frame.payload.as_ref(), version) else {
            return;
        };
        let Some(index) = self
            .keepalives
            .iter()
            .position(|(id, _)| *id == keepalive.id)
        else {
            return;
        };
        let sent_at = self.keepalives[index].1;
        self.keepalives.drain(..=index);
        session.client_state().record_ping(sent_at.elapsed());
    }

    const fn client_open(&self, client: &ClientBridge, in_game: bool) -> bool {
        match client.state() {
            ConnectionState::Play => in_game,
            ConnectionState::Config => !self.config_closing,
            _ => false,
        }
    }
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
pub async fn proxy_loop(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    backend: &mut BackendBridge,
    server: &ServerId,
    join: &mut Option<ServerJoin>,
) -> ProxyLoopOutcome {
    let in_game = io.client.state() == ConnectionState::Play && join.is_none();
    Loop {
        ids: HotIds::resolve(ctx.registry(), ctx.version()),
        chain: FrameChain::new(ctx, server),
        ctx,
        io,
        backend,
        server,
        state: LoopState::new(),
        in_game,
        backend_tree: None,
        join,
    }
    .run()
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Client,
    Backend,
}

enum Step {
    Continue,
    Kicked(Box<BackendKick>),
    Milestone(Milestone),
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

struct Loop<'a> {
    ctx: &'a SessionContext<'a>,
    io: &'a mut SessionIo,
    backend: &'a mut BackendBridge,
    server: &'a ServerId,
    ids: HotIds,
    chain: FrameChain<'a>,
    state: LoopState,
    in_game: bool,
    backend_tree: Option<CCommands>,
    join: &'a mut Option<ServerJoin>,
}

impl Loop<'_> {
    async fn run(mut self) -> ProxyLoopOutcome {
        let registry = self.ctx.registry();
        let mut tree_updates = self.ctx.services.command_manager.subscribe();
        let mut permission_updates = self.ctx.session.subscribe_permissions();
        if self.in_game {
            let outcome = self
                .io
                .commands
                .drain(&mut self.io.client, registry, self.in_game);
            self.deliver();
            if let Err(e) = self.io.client.flush().await {
                return ProxyLoopOutcome::Error(e);
            }
            if let Err(e) = self.backend.flush().await {
                return ProxyLoopOutcome::Error(e);
            }
            if let Some(end) = self.settle(outcome).await {
                return end;
            }
        }
        loop {
            let event = tokio::select! {
                biased;
                Some(command) = self.io.commands.recv() => LoopEvent::Command(command),
                () = self.ctx.token.cancelled() => LoopEvent::Shutdown,
                Ok(()) = tree_updates.changed() => LoopEvent::CommandsChanged,
                Ok(()) = permission_updates.changed() => LoopEvent::CommandsChanged,
                event = next_frame(&mut self.io.client, &mut *self.backend) => event,
            };
            match event {
                LoopEvent::Command(command) => {
                    let outcome = match self.io.commands.apply(
                        command,
                        &mut self.io.client,
                        registry,
                        self.in_game,
                    ) {
                        CommandOutcome::Continue => {
                            self.io
                                .commands
                                .drain(&mut self.io.client, registry, self.in_game)
                        }
                        outcome => outcome,
                    };
                    self.deliver();
                    if let Err(e) = self.io.client.flush().await {
                        tracing::warn!("failed to flush player command: {e}");
                    }
                    if let Err(e) = self.backend.flush().await {
                        tracing::warn!("failed to flush a plugin message to the backend: {e}");
                    }
                    if let Some(end) = self.settle(outcome).await {
                        break end;
                    }
                }
                LoopEvent::CommandsChanged => {
                    if let Some(tree) = self.backend_tree.as_ref()
                        && self.io.client.state() == ConnectionState::Play
                        && let Some(frame) = command_tree_frame(tree, self.ctx, &self.ids)
                    {
                        if let Err(e) = self.io.client.queue_frame(&frame) {
                            tracing::warn!("failed to queue the refreshed command tree: {e}");
                        }
                        if let Err(e) = self.io.client.flush().await {
                            tracing::warn!("failed to flush the refreshed command tree: {e}");
                        }
                    }
                }
                LoopEvent::Shutdown => {
                    if let Some(reason) =
                        self.io
                            .commands
                            .take_kick(&mut self.io.client, registry, self.in_game)
                    {
                        break kick(&mut self.io.client, &reason, registry).await;
                    }
                    let _ = self.io.client.flush().await;
                    break ProxyLoopOutcome::Shutdown;
                }
                LoopEvent::Client(Ok(Some(frame))) => {
                    if let Some(end) = self.pump(frame, Side::Client).await {
                        break end;
                    }
                }
                LoopEvent::Client(Ok(None)) => break ProxyLoopOutcome::ClientDisconnected,
                LoopEvent::Client(Err(e)) => break ProxyLoopOutcome::Error(e),
                LoopEvent::Backend(Ok(Some(frame))) => {
                    if let Some(end) = self.pump(frame, Side::Backend).await {
                        break end;
                    }
                }
                LoopEvent::Backend(Ok(None)) => {
                    break ProxyLoopOutcome::BackendDisconnected { reason: None };
                }
                LoopEvent::Backend(Err(e)) if e.is_expected_disconnect() => {
                    let _ = self.io.client.flush().await;
                    break ProxyLoopOutcome::BackendDisconnected {
                        reason: Some(e.to_string()),
                    };
                }
                LoopEvent::Backend(Err(e)) => break ProxyLoopOutcome::Error(e),
            }
        }
    }

    async fn pump(&mut self, first: PacketFrame, side: Side) -> Option<ProxyLoopOutcome> {
        let registry = self.ctx.registry();
        let mut step = Ok(Step::Continue);
        let mut commands = CommandOutcome::Continue;
        let mut next = Some(first);
        while let Some(frame) = next.take() {
            step = match side {
                Side::Client => self.on_client(frame).await.map(|()| Step::Continue),
                Side::Backend => self.on_backend(frame).await,
            };
            commands = self
                .io
                .commands
                .drain(&mut self.io.client, registry, self.in_game);
            if !matches!(step, Ok(Step::Continue)) || !matches!(commands, CommandOutcome::Continue)
            {
                break;
            }
            let buffered = match side {
                Side::Client => self.io.client.try_next_frame(),
                Side::Backend => self.backend.try_next_frame(),
            };
            next = match buffered {
                Ok(frame) => frame,
                Err(e) => {
                    step = Err(e);
                    None
                }
            };
        }
        self.deliver();
        let step = match step {
            Ok(step) => step,
            Err(e) => {
                let _ = self.backend.flush().await;
                let _ = self.io.client.flush().await;
                return Some(ended(e, side, side));
            }
        };
        if matches!(step, Step::Milestone(Milestone::Joined)) {
            self.announce_channels();
        }
        if let Some(end) = self.flush(side).await {
            return Some(end);
        }
        match step {
            Step::Continue => {}
            Step::Milestone(milestone) => {
                reach(milestone, &mut *self.join, self.ctx.services).await
            }
            Step::Kicked(kick) => return Some(ProxyLoopOutcome::BackendKick(kick)),
        }
        self.settle(commands).await
    }

    async fn flush(&mut self, side: Side) -> Option<ProxyLoopOutcome> {
        let order = match side {
            Side::Client => [Side::Backend, Side::Client],
            Side::Backend => [Side::Client, Side::Backend],
        };
        for peer in order {
            let flushed = match peer {
                Side::Client => self.io.client.flush().await,
                Side::Backend => self.backend.flush().await,
            };
            if let Err(e) = flushed {
                let _ = self.backend.flush().await;
                let _ = self.io.client.flush().await;
                return Some(ended(e, peer, side));
            }
        }
        None
    }

    fn deliver(&mut self) {
        let open = self.state.client_open(&self.io.client, self.in_game);
        self.io.commands.deliver_messages(
            &mut self.io.client,
            Some(&mut *self.backend),
            self.ctx.registry(),
            open,
        );
    }

    async fn settle(&mut self, outcome: CommandOutcome) -> Option<ProxyLoopOutcome> {
        match outcome {
            CommandOutcome::Continue => None,
            CommandOutcome::Kick(reason) => {
                Some(kick(&mut self.io.client, &reason, self.ctx.registry()).await)
            }
            CommandOutcome::Switch(target, cause) => {
                Some(ProxyLoopOutcome::SwitchRequested { target, cause })
            }
        }
    }

    fn restore_presentation(&mut self) {
        if !std::mem::take(&mut self.state.presentation_lost) {
            return;
        }
        self.io.commands.discard_deferred_presentation();
        let frames = match presentation::restore_frames(
            &self.ctx.session,
            self.ctx.registry(),
            self.io.client.protocol_version,
        ) {
            Ok(frames) => frames,
            Err(e) => {
                tracing::warn!("failed to rebuild the player list and boss bars: {e}");
                return;
            }
        };
        for frame in &frames {
            if let Err(e) = self.io.client.queue_frame(frame) {
                tracing::warn!("failed to restore the player list and boss bars: {e}");
                return;
            }
        }
    }

    fn announce_channels(&mut self) {
        let services = self.ctx.services;
        let version = self.ctx.version();
        let config = services
            .domain_router
            .find_by_server_id(self.server.as_str());
        let names = services
            .plugin_messaging
            .announced_channels(config.as_deref(), version);
        if names.is_empty() {
            return;
        }
        let Some(id) =
            MessageIds::resolve(self.ctx.registry(), version).serverbound(self.backend.state)
        else {
            return;
        };
        let register = ChannelId::register();
        let channel = register.wire_name(ApiVersion::new(version.0));
        for payload in channels::channel_payloads(names.iter().map(String::as_str)) {
            if let Err(e) = self
                .backend
                .queue_frame(&channels::build(id, channel, &payload, version))
            {
                tracing::warn!("failed to announce the proxy's plugin channels: {e}");
            }
        }
    }

    async fn on_client(&mut self, mut frame: PacketFrame) -> Result<(), CoreError> {
        let registry = self.ctx.registry();
        let services = self.ctx.services;
        let session = &self.ctx.session;
        let version = self.io.client.protocol_version;
        let state = LoopState::client_reading(&self.io.client);

        if state == ConnectionState::Play {
            if self.state.reconfiguring && Some(frame.id) == self.ids.s_ack_config {
                self.backend.queue_frame(&frame)?;
                self.backend.set_state(ConnectionState::Config);
                self.io.client.reconfiguration_acknowledged();
                self.state.reconfiguring = false;
                self.io
                    .client_codec
                    .notify_state_change(protocol_state_to_api(ConnectionState::Config));
                tracing::debug!("state transition: Play → Config (AcknowledgeConfiguration)");
                return Ok(());
            }

            if apply_codec_filter(&mut self.io.client_codec, &mut frame, &mut *self.backend)? {
                return Ok(());
            }

            if Some(frame.id) == self.ids.s_keepalive {
                self.state.keepalive_answered(session, &frame, version);
            }
            match self.chain.serverbound(frame, state).await {
                Some(next) => frame = next,
                None => return Ok(()),
            }

            if Some(frame.id) == self.ids.s_chat_session {
                tracing::debug!("dropping Chat Session Update (offline backend)");
                return Ok(());
            }

            if Some(frame.id) == self.ids.s_tab_request
                && let Some(packet_id) = self.ids.c_tab_response
                && let Ok(DecodedPacket::Typed { id: _, packet }) =
                    registry.decode_frame(&frame, state, Direction::Serverbound, version)
                && let Some(req) = packet.as_any().downcast_ref::<STabCompleteRequest>()
                && let Some(input) = req.text.trim_start().strip_prefix('/')
            {
                let reply = TabReply::new(packet_id, req, version);
                let source = CommandSource::Player(self.ctx.player());
                match services.command_manager.prepare_suggestion(source, input) {
                    Prepared::Unknown => {}
                    Prepared::Denied => {
                        if let Some(answer) = reply.frame(Vec::new()) {
                            self.io.client.queue_frame(&answer)?;
                            return Ok(());
                        }
                    }
                    Prepared::Ready(completion) => {
                        let replying = Arc::clone(session);
                        session.run_completion(completion, move |suggestions| {
                            let Some(answer) = reply.frame(suggestions) else {
                                return;
                            };
                            if let Err(e) =
                                replying.send_packet(RawPacket::new(answer.id, answer.payload))
                            {
                                tracing::debug!("dropping proxy command suggestions: {e}");
                            }
                        });
                        return Ok(());
                    }
                }
            }

            if let Some(input) = decode_player_input(&frame, &self.ids.chat, version) {
                let scope = ChatScope {
                    ctx: self.ctx,
                    ids: &self.ids.chat,
                    server: self.server,
                };
                match intercept(
                    input,
                    frame,
                    &scope,
                    &mut self.io.client,
                    &mut *self.backend,
                )
                .await?
                {
                    Some(next) => frame = next,
                    None => return Ok(()),
                }
            }

            match self
                .chain
                .raw_event(frame, Direction::Serverbound, state)
                .await
            {
                Some(next) => frame = next,
                None => return Ok(()),
            }

            self.backend.queue_frame(&frame)?;
            return Ok(());
        }

        if state == ConnectionState::Config {
            match self.chain.serverbound(frame, state).await {
                Some(next) => frame = next,
                None => return Ok(()),
            }
        }

        match registry.decode_frame(&frame, state, Direction::Serverbound, version) {
            Ok(DecodedPacket::Typed { packet, .. }) => {
                if packet
                    .as_any()
                    .downcast_ref::<SLoginAcknowledged>()
                    .is_some()
                {
                    self.backend.queue_frame(&frame)?;
                    self.io.client.set_state(ConnectionState::Config);
                    self.backend.set_state(ConnectionState::Config);
                    self.io
                        .client_codec
                        .notify_state_change(protocol_state_to_api(ConnectionState::Config));
                    tracing::debug!("state transition: Login → Config (LoginAcknowledged)");
                    return Ok(());
                }

                if packet
                    .as_any()
                    .downcast_ref::<SAcknowledgeFinishConfig>()
                    .is_some()
                {
                    self.backend.queue_frame(&frame)?;
                    self.io.client.set_state(ConnectionState::Play);
                    self.backend.set_state(ConnectionState::Play);
                    self.state.config_closing = false;
                    self.io
                        .client_codec
                        .notify_state_change(protocol_state_to_api(ConnectionState::Play));
                    tracing::debug!("state transition: Config → Play (AcknowledgeFinishConfig)");
                    return Ok(());
                }

                self.backend.queue_frame(&frame)?;
            }
            Ok(DecodedPacket::Opaque { .. }) | Err(_) => {
                self.backend.queue_frame(&frame)?;
            }
        }

        Ok(())
    }

    async fn on_backend(&mut self, frame: PacketFrame) -> Result<Step, CoreError> {
        let joins = self.ids.joins_game(
            &frame,
            self.state.backend_reading(self.backend),
            self.in_game,
        );
        let milestone = self
            .ids
            .milestone(&frame, self.backend, self.join.is_some());
        let action = self.forward_backend(frame).await?;
        self.in_game |= joins;
        self.in_game &= !self.state.reconfiguring;
        if joins {
            self.restore_presentation();
        }
        Ok(match action {
            BackendAction::Kicked(kick) => Step::Kicked(kick),
            BackendAction::Continue => milestone.map_or(Step::Continue, Step::Milestone),
        })
    }

    async fn forward_backend(
        &mut self,
        mut frame: PacketFrame,
    ) -> Result<BackendAction, CoreError> {
        let registry = self.ctx.registry();
        let services = self.ctx.services;
        let version = self.io.client.protocol_version;
        let state = self.state.backend_reading(self.backend);

        if state == ConnectionState::Play {
            if apply_codec_filter(&mut self.io.server_codec, &mut frame, &mut self.io.client)? {
                return Ok(BackendAction::Continue);
            }

            if Some(frame.id) == self.ids.c_keepalive {
                self.state.keepalive_sent(&frame, version);
            }
            match self
                .chain
                .clientbound(frame, &mut *self.backend, state)
                .await?
            {
                Some(next) => frame = next,
                None => return Ok(BackendAction::Continue),
            }
            match self
                .chain
                .raw_event(frame, Direction::Clientbound, state)
                .await
            {
                Some(next) => frame = next,
                None => return Ok(BackendAction::Continue),
            }

            if Some(frame.id) == self.ids.c_disconnect {
                return Ok(BackendAction::Kicked(Box::new(BackendKick::new(
                    frame,
                    ConnectionState::Play,
                    version,
                ))));
            }
            if Some(frame.id) == self.ids.c_start_config {
                self.io.client.queue_frame(&frame)?;
                self.io.client.begin_reconfiguration();
                self.state.reconfiguring = true;
                self.state.presentation_lost = true;
                tracing::debug!("state transition: Play → Config (backend StartConfiguration)");
                return Ok(BackendAction::Continue);
            }
            let intercepted =
                Some(frame.id) == self.ids.c_commands && services.config.announce_proxy_commands;
            if !intercepted {
                self.io.client.queue_frame(&frame)?;
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
                .and_then(|tree| command_tree_frame(tree, self.ctx, &self.ids));
            self.io
                .client
                .queue_frame(injected.as_ref().unwrap_or(&frame))?;
            if tree.is_some() {
                self.backend_tree = tree;
            }
            return Ok(BackendAction::Continue);
        }

        if state == ConnectionState::Config {
            match self
                .chain
                .clientbound(frame, &mut *self.backend, state)
                .await?
            {
                Some(next) => frame = next,
                None => return Ok(BackendAction::Continue),
            }
        }

        match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
            Ok(DecodedPacket::Typed { packet, .. }) => {
                if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                    let threshold = set_comp.threshold.0;
                    self.backend.set_compression(threshold);
                    self.io.client.queue_frame(&frame)?;
                    self.io.client.set_compression(threshold);
                    match self.io.client.compression_threshold() {
                        Some(effective) => {
                            self.io.server_codec.notify_compression_change(effective);
                            tracing::debug!(threshold = effective, "compression activated");
                        }
                        None => {
                            tracing::debug!(threshold, "compression left disabled by backend");
                        }
                    }
                    return Ok(BackendAction::Continue);
                }

                if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                    self.io.client.queue_frame(&frame)?;
                    if version.less_than(ProtocolVersion::V1_20_2) {
                        self.io.client.set_state(ConnectionState::Play);
                        self.backend.set_state(ConnectionState::Play);
                        self.io
                            .server_codec
                            .notify_state_change(protocol_state_to_api(ConnectionState::Play));
                        tracing::debug!("state transition: Login → Play (pre-1.20.2)");
                    }
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

                if packet.as_any().downcast_ref::<CFinishConfig>().is_some() {
                    services.registry_codec_cache.finalize(version);
                    self.io.client.queue_frame(&frame)?;
                    self.state.config_closing = true;
                    return Ok(BackendAction::Continue);
                }

                if state == ConnectionState::Config {
                    services
                        .registry_codec_cache
                        .collect_config_frame(registry, version, &frame);
                }

                self.io.client.queue_frame(&frame)?;
            }
            Ok(DecodedPacket::Opaque { .. }) => {
                if state == ConnectionState::Config {
                    services
                        .registry_codec_cache
                        .collect_config_frame(registry, version, &frame);
                }
                self.io.client.queue_frame(&frame)?;
            }
            Err(e) => {
                tracing::warn!("failed to decode backend frame: {e}");
                self.io.client.queue_frame(&frame)?;
            }
        }

        Ok(BackendAction::Continue)
    }
}

/// Action to take after processing a backend → client packet.
#[derive(Debug)]
enum BackendAction {
    /// Continue the loop normally.
    Continue,
    Kicked(Box<BackendKick>),
}

fn ended(error: CoreError, peer: Side, side: Side) -> ProxyLoopOutcome {
    if side == Side::Backend || !error.is_expected_disconnect() {
        return ProxyLoopOutcome::Error(error);
    }
    match peer {
        Side::Client => ProxyLoopOutcome::ClientDisconnected,
        Side::Backend => ProxyLoopOutcome::BackendDisconnected {
            reason: Some(error.to_string()),
        },
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
    ctx: &SessionContext<'_>,
    hot_ids: &HotIds,
) -> Option<PacketFrame> {
    let id = hot_ids.c_commands?;
    let services = ctx.services;
    let version = ctx.version();
    let source = CommandSource::Player(ctx.player());
    let proxy_tree = services.command_manager.tree_for(Some(&source));
    let visible = services.permission_service.visible_subcommands(&source);
    let mut modified = tree.clone();
    if let Err(e) = crate::commands::brigadier::inject_proxy_commands(
        &mut modified,
        version,
        &proxy_tree,
        Some(&visible),
    ) {
        tracing::warn!("failed to inject proxy commands into CCommands: {e}");
        return None;
    }
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
        FilterResult::Pass { modified } => {
            if modified {
                *frame = raw_to_frame(&raw);
            }
            Ok(false)
        }
        FilterResult::Dropped => Ok(true),
        FilterResult::Replaced(mut output) => {
            send_injected_frames(writer, &mut output, true, true)?;
            Ok(true) // Original frame is NOT sent
        }
        FilterResult::PassWithInjections {
            mut output,
            modified,
        } => {
            send_injected_frames(writer, &mut output, true, false)?;
            if modified {
                *frame = raw_to_frame(&raw);
            }
            writer.queue_frame(frame)?;
            send_injected_frames(writer, &mut output, false, true)?;
            Ok(true) // Frame already queued with injections
        }
    }
}

struct TabReply {
    packet_id: i32,
    transaction_id: i32,
    start: i32,
    length: i32,
    version: ProtocolVersion,
}

impl TabReply {
    fn new(packet_id: i32, request: &STabCompleteRequest, version: ProtocolVersion) -> Self {
        let text = request.text.as_str();
        let start = text.rfind(' ').map_or(0, |i| i + 1);
        Self {
            packet_id,
            transaction_id: request.transaction_id,
            start: i32::try_from(start).unwrap_or(i32::MAX),
            length: i32::try_from(text.len() - start).unwrap_or(i32::MAX),
            version,
        }
    }

    fn frame(&self, suggestions: Vec<Suggestion>) -> Option<PacketFrame> {
        let response = CTabCompleteResponse {
            transaction_id: self.transaction_id,
            start: self.start,
            length: self.length,
            matches: suggestions
                .into_iter()
                .map(|suggestion| TabCompleteMatch {
                    text: suggestion.text,
                    tooltip: suggestion.tooltip.map(|tooltip| {
                        encode_text_component(&tooltip, self.version, ConnectionState::Play)
                    }),
                })
                .collect(),
        };
        let mut buf = Vec::new();
        match infrarust_protocol::packets::Packet::encode(&response, &mut buf, self.version) {
            Ok(()) => Some(PacketFrame::new(self.packet_id, buf.into())),
            Err(e) => {
                tracing::warn!("failed to encode proxy command suggestions: {e}");
                None
            }
        }
    }
}
