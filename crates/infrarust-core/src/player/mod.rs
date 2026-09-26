//! Player session management.
//!
//! Provides [`PlayerSession`] (the concrete implementation of `dyn Player`)
//! and [`PlayerCommand`] (the command channel enum for packet injection).

mod api;
pub(crate) mod client_state;
pub(crate) mod command_queue;
pub(crate) mod commands;
pub(crate) mod lifecycle;
pub(crate) mod packets;
pub mod registry;
mod transfer;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;

use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use infrarust_api::error::PlayerError;
use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::messaging::ChannelId;
use infrarust_api::permissions::{DefaultPermissionChecker, PermissionChecker, PermissionSubject};
use infrarust_api::player::{
    BossBar, BossBarUpdate, ConnectionResult, Player, ResourcePackRequest, session_task,
};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};
use infrarust_config::ServerAddress;
use infrarust_protocol::version::ProtocolVersion as WireVersion;

use crate::event_bus::EventBusImpl;
use crate::loadbalancer::BackendLoad;
use crate::permissions::PermissionService;
use crate::util::sync::{lock, read, write};

use crate::session::presentation::Presentation;
use client_state::ClientState;
use command_queue::CommandQueue;

const TAB_LIST_SINCE: WireVersion = WireVersion::V1_8;
const BOSS_BAR_SINCE: WireVersion = WireVersion::V1_9;
const RESOURCE_PACK_SINCE: WireVersion = WireVersion::V1_8;
const PACK_STACK_SINCE: WireVersion = WireVersion::V1_20_3;
const COOKIES_SINCE: WireVersion = WireVersion::V1_20_5;
const TRANSFER_SINCE: WireVersion = WireVersion::V1_20_5;
const MAX_TRANSFER_HOST: usize = 32_767;

/// Channel buffer size for player commands.
const COMMAND_CHANNEL_SIZE: usize = 32;

const SELF_WAIT_WARN_INTERVAL: Duration = Duration::from_secs(10);

pub const SHUTDOWN_REASON: &str = "Proxy is shutting down";

/// Commands sent to the proxy loop for a specific player.
#[derive(Debug)]
pub enum PlayerCommand {
    /// Send a system chat message to the player.
    SendMessage(Component),
    /// Display a title on the player's screen.
    SendTitle(Box<TitleData>),
    /// Display a message in the action bar.
    SendActionBar(Component),
    /// Send a raw packet to the player's client.
    SendPacket(RawPacket),
    /// Kick the player with a reason.
    Kick(Component),
    /// Switch the player to a different backend server.
    SwitchServer(ServerId, ConnectCause),
    PluginMessage(OutgoingMessage),
    HeaderFooter(Box<(Component, Component)>),
    ClearTitle {
        reset: bool,
    },
    BossBar(Uuid, BossBarCommand),
    Client(ClientCommand),
}

#[derive(Debug)]
pub enum BossBarCommand {
    Show(Box<BossBar>),
    Update(BossBarUpdate),
    Hide,
}

#[derive(Debug)]
pub enum ClientCommand {
    PushPack(Box<ResourcePackRequest>),
    PopPack(Option<Uuid>),
    StoreCookie {
        key: String,
        data: Bytes,
    },
    RequestCookie {
        key: String,
        reply: oneshot::Sender<Option<Bytes>>,
    },
    Transfer {
        host: String,
        port: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageTarget {
    Client,
    Backend,
}

#[derive(Debug)]
pub struct OutgoingMessage {
    pub target: MessageTarget,
    pub channel: ChannelId,
    pub data: Bytes,
}

struct Routing {
    current: Option<ServerId>,
    previous: Option<ServerId>,
    pending: Option<ServerId>,
}

#[derive(Default)]
struct WarnWindow {
    last: Option<Instant>,
    suppressed: u64,
}

impl WarnWindow {
    fn admit(&mut self, now: Instant) -> Option<u64> {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < SELF_WAIT_WARN_INTERVAL)
        {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.last = Some(now);
        Some(std::mem::take(&mut self.suppressed))
    }
}

/// Concrete implementation of [`Player`].
///
/// Holds identity data and a command channel to the proxy loop.
/// Sync methods (`send_message`, etc.) use `try_send` on the bounded channel.
/// `switch_server` uses `send().await`, and `try_send` from the player's own session.
pub struct PlayerSession {
    player_id: PlayerId,
    profile: GameProfile,
    protocol_version: ProtocolVersion,
    remote_addr: SocketAddr,
    routing: RwLock<Routing>,
    connected_address: RwLock<Option<ServerAddress>>,
    backend_load: Arc<BackendLoad>,
    connected: AtomicBool,
    active: bool,
    online_mode: bool,
    connected_at: SystemTime,
    command_tx: mpsc::Sender<PlayerCommand>,
    shutdown_token: CancellationToken,
    permission_checker: RwLock<Arc<dyn PermissionChecker>>,
    permission_override: AtomicBool,
    permissions: Option<Arc<PermissionService>>,
    permissions_changed: watch::Sender<u64>,
    virtual_host: Option<String>,
    released: CancellationToken,
    client: ClientState,
    presentation: Arc<Presentation>,
    events: Option<Arc<EventBusImpl>>,
    shared: Weak<Self>,
    connects: Mutex<Vec<(ServerId, oneshot::Sender<ConnectionResult>)>>,
    typed_commands: CommandQueue,
    self_waits: Mutex<WarnWindow>,
}

impl std::fmt::Debug for PlayerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerSession")
            .field("player_id", &self.player_id)
            .field("username", &self.profile.username)
            .field("active", &self.active)
            .field("connected", &self.connected.load(Ordering::Acquire))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Intercepted { online_mode: bool },
    Forwarded,
}

impl SessionKind {
    const fn active(self) -> bool {
        matches!(self, Self::Intercepted { .. })
    }

    pub(crate) const fn online_mode(self) -> bool {
        match self {
            Self::Intercepted { online_mode } => online_mode,
            Self::Forwarded => false,
        }
    }
}

pub struct PlayerSessionBuilder {
    player_id: PlayerId,
    profile: GameProfile,
    protocol_version: ProtocolVersion,
    remote_addr: SocketAddr,
    command_tx: mpsc::Sender<PlayerCommand>,
    shutdown_token: CancellationToken,
    backend_load: Arc<BackendLoad>,
    kind: SessionKind,
    current_server: Option<ServerId>,
    permission_checker: Arc<dyn PermissionChecker>,
    permissions: Option<Arc<PermissionService>>,
    virtual_host: Option<String>,
    events: Option<Arc<EventBusImpl>>,
}

impl PlayerSessionBuilder {
    #[must_use]
    pub const fn kind(mut self, kind: SessionKind) -> Self {
        self.kind = kind;
        self
    }

    #[must_use]
    pub fn current_server(mut self, server: ServerId) -> Self {
        self.current_server = Some(server);
        self
    }

    #[must_use]
    pub fn permission_checker(mut self, checker: Arc<dyn PermissionChecker>) -> Self {
        self.permission_checker = checker;
        self
    }

    #[must_use]
    pub fn permissions(mut self, permissions: Arc<PermissionService>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    #[must_use]
    pub fn virtual_host(mut self, host: impl Into<String>) -> Self {
        self.virtual_host = Some(host.into());
        self
    }

    #[must_use]
    pub fn events(mut self, events: Arc<EventBusImpl>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn build(self) -> Arc<PlayerSession> {
        let typed_commands = CommandQueue::new(self.profile.username.clone());
        Arc::new_cyclic(|shared| PlayerSession {
            player_id: self.player_id,
            profile: self.profile,
            protocol_version: self.protocol_version,
            remote_addr: self.remote_addr,
            routing: RwLock::new(Routing {
                current: self.current_server,
                previous: None,
                pending: None,
            }),
            connected_address: RwLock::new(None),
            backend_load: self.backend_load,
            connected: AtomicBool::new(true),
            active: self.kind.active(),
            online_mode: self.kind.online_mode(),
            connected_at: SystemTime::now(),
            command_tx: self.command_tx,
            shutdown_token: self.shutdown_token,
            permission_checker: RwLock::new(self.permission_checker),
            permission_override: AtomicBool::new(false),
            permissions: self.permissions,
            permissions_changed: watch::Sender::new(0),
            virtual_host: self.virtual_host,
            released: CancellationToken::new(),
            client: ClientState::default(),
            presentation: Arc::default(),
            events: self.events,
            shared: Weak::clone(shared),
            connects: Mutex::new(Vec::new()),
            typed_commands,
            self_waits: Mutex::default(),
        })
    }
}

impl PlayerSession {
    pub fn builder(
        player_id: PlayerId,
        profile: GameProfile,
        protocol_version: ProtocolVersion,
        remote_addr: SocketAddr,
        command_tx: mpsc::Sender<PlayerCommand>,
        shutdown_token: CancellationToken,
        backend_load: Arc<BackendLoad>,
    ) -> PlayerSessionBuilder {
        PlayerSessionBuilder {
            player_id,
            profile,
            protocol_version,
            remote_addr,
            command_tx,
            shutdown_token,
            backend_load,
            kind: SessionKind::Intercepted { online_mode: false },
            current_server: None,
            permission_checker: Arc::new(DefaultPermissionChecker),
            permissions: None,
            virtual_host: None,
            events: None,
        }
    }

    fn shared_player(&self) -> Option<Arc<dyn Player>> {
        self.shared
            .upgrade()
            .map(|session| session as Arc<dyn Player>)
    }

    pub(crate) const fn presentation(&self) -> &Arc<Presentation> {
        &self.presentation
    }

    pub(crate) fn settle_connect(&self, target: &ServerId, result: &ConnectionResult) {
        let settled: Vec<_> = {
            let mut connects = lock(&self.connects);
            let (settled, waiting) = std::mem::take(&mut *connects)
                .into_iter()
                .partition(|(server, _)| server == target);
            *connects = waiting;
            settled
        };
        for (_, reply) in settled {
            let _ = reply.send(result.clone());
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_test(active: bool) -> (Arc<Self>, mpsc::Receiver<PlayerCommand>) {
        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_SIZE);
        let kind = if active {
            SessionKind::Intercepted { online_mode: false }
        } else {
            SessionKind::Forwarded
        };
        let session = Self::builder(
            PlayerId::new(1),
            GameProfile {
                uuid: Uuid::new_v4(),
                username: "TestPlayer".to_string(),
                properties: vec![],
            },
            ProtocolVersion::new(767),
            SocketAddr::from(([127, 0, 0, 1], 12345)),
            tx,
            CancellationToken::new(),
            Arc::new(BackendLoad::new()),
        )
        .kind(kind)
        .build();
        (session, rx)
    }

    pub fn channel() -> (mpsc::Sender<PlayerCommand>, mpsc::Receiver<PlayerCommand>) {
        mpsc::channel(COMMAND_CHANNEL_SIZE)
    }

    /// Marks the player as disconnected (called by handlers during cleanup).
    pub fn set_disconnected(&self) {
        self.connected.store(false, Ordering::Release);
        self.set_connected_address(None);
        self.presentation.end();
        lock(&self.connects).clear();
        drop(self.typed_commands.stop());
    }

    /// Updates the current server (called by the proxy loop on server switch).
    pub fn set_current_server(&self, server: ServerId) {
        let mut routing = write(&self.routing);
        if routing.current.as_ref() != Some(&server) {
            routing.previous = routing.current.replace(server);
        }
        routing.pending = None;
    }

    pub(crate) fn previous_server(&self) -> Option<ServerId> {
        read(&self.routing).previous.clone()
    }

    pub(crate) fn set_pending_server(&self, server: ServerId) {
        write(&self.routing).pending = Some(server);
    }

    pub(crate) fn counted_server(&self) -> Option<ServerId> {
        let routing = read(&self.routing);
        routing.current.clone().or_else(|| routing.pending.clone())
    }

    pub fn set_connected_address(&self, address: Option<ServerAddress>) {
        let mut guard = write(&self.connected_address);
        if *guard == address {
            return;
        }
        if let Some(next) = &address {
            self.backend_load.acquire(next);
        }
        if let Some(previous) = std::mem::replace(&mut *guard, address) {
            self.backend_load.release(&previous);
        }
    }

    pub fn connected_address(&self) -> Option<ServerAddress> {
        read(&self.connected_address).clone()
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown_token
    }

    pub fn game_profile(&self) -> &GameProfile {
        &self.profile
    }

    pub(crate) const fn client_state(&self) -> &ClientState {
        &self.client
    }

    pub(crate) fn request_switch(
        &self,
        target: ServerId,
        cause: ConnectCause,
    ) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SwitchServer(target, cause))
    }

    fn send_message_to(
        &self,
        target: MessageTarget,
        channel: &ChannelId,
        data: Bytes,
        max: usize,
    ) -> Result<(), PlayerError> {
        if !self.active {
            return Err(PlayerError::NotActive);
        }
        if data.len() > max {
            return Err(PlayerError::MessageTooLarge {
                size: data.len(),
                max,
            });
        }
        self.try_send_command(PlayerCommand::PluginMessage(OutgoingMessage {
            target,
            channel: channel.clone(),
            data,
        }))
    }

    pub fn permission_subject(&self) -> PermissionSubject {
        let subject = PermissionSubject::player(
            self.player_id,
            self.profile.clone(),
            self.online_mode,
            self.remote_addr,
        );
        match &self.virtual_host {
            Some(host) => subject.with_virtual_host(host.clone()),
            None => subject,
        }
    }

    pub fn set_permission_checker(&self, checker: Arc<dyn PermissionChecker>) {
        *write(&self.permission_checker) = checker;
    }

    pub fn override_permission_checker(&self, checker: Arc<dyn PermissionChecker>) {
        self.permission_override.store(true, Ordering::Release);
        self.set_permission_checker(checker);
    }

    pub fn subscribe_permissions(&self) -> watch::Receiver<u64> {
        self.permissions_changed.subscribe()
    }

    pub(crate) async fn setup_permissions(self: &Arc<Self>, bus: &EventBusImpl) {
        if let Some(permissions) = &self.permissions {
            let checker = permissions.create_checker(&self.permission_subject()).await;
            self.set_permission_checker(checker);
        }
        let setup = bus
            .fire(PermissionsSetupEvent::new(
                Arc::clone(self) as Arc<dyn Player>,
                self.online_mode,
            ))
            .await;
        if let PermissionsSetupResult::Custom(checker) = setup.result() {
            self.override_permission_checker(Arc::clone(checker));
        }
    }

    fn permission_checker(&self) -> Arc<dyn PermissionChecker> {
        Arc::clone(&read(&self.permission_checker))
    }

    pub(crate) fn mark_released(&self) {
        self.released.cancel();
    }

    pub(crate) async fn released(&self) {
        self.released.cancelled().await;
    }

    /// Checks preconditions for sending commands and sends via `try_send`.
    fn try_send_command(&self, cmd: PlayerCommand) -> Result<(), PlayerError> {
        self.ready()?;
        self.command_tx
            .try_send(cmd)
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    async fn send_command(&self, cmd: PlayerCommand) -> Result<(), PlayerError> {
        if self.runs_this_code() {
            return self.try_send_command(cmd);
        }
        self.ready()?;
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
    }

    fn runs_this_code(&self) -> bool {
        session_task::current() == Some(self.player_id)
    }

    fn refuse_self_wait(&self, call: &'static str) -> Result<(), PlayerError> {
        if !self.runs_this_code() {
            return Ok(());
        }
        let admitted = lock(&self.self_waits).admit(Instant::now());
        if let Some(suppressed) = admitted {
            tracing::warn!(
                player = %self.profile.username,
                call,
                suppressed,
                "a plugin awaited a call that needs the player's session from code that session is running; the call fails at once instead of waiting on itself. Use switch_server, or await the call in a task of its own"
            );
        }
        Err(PlayerError::WouldDeadlock)
    }

    fn ready(&self) -> Result<(), PlayerError> {
        if !self.active {
            return Err(PlayerError::NotActive);
        }
        if !self.connected.load(Ordering::Acquire) {
            return Err(PlayerError::Disconnected);
        }
        Ok(())
    }

    fn supports(&self, since: WireVersion, feature: &str) -> Result<(), PlayerError> {
        self.ready()?;
        let version = WireVersion(self.protocol_version.raw());
        if version.less_than(since) {
            return Err(PlayerError::Unsupported(format!(
                "{feature}: the client runs Minecraft {version}, this needs {since} or newer"
            )));
        }
        Ok(())
    }
}

impl Drop for PlayerSession {
    fn drop(&mut self) {
        if let Some(address) = self
            .connected_address
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.backend_load.release(&address);
        }
    }
}
