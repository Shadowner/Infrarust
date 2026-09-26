//! Player session management.
//!
//! Provides [`PlayerSession`] (the concrete implementation of `dyn Player`)
//! and [`PlayerCommand`] (the command channel enum for packet injection).

pub(crate) mod client_state;
pub(crate) mod command_queue;
pub(crate) mod commands;
pub(crate) mod lifecycle;
pub(crate) mod packets;
pub(crate) mod presentation;
pub mod registry;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;

use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use infrarust_api::error::PlayerError;
use infrarust_api::event::{BoxFuture, ResultedEvent};
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};
use infrarust_api::messaging::{ChannelId, MAX_TO_BACKEND_PAYLOAD, MAX_TO_CLIENT_PAYLOAD};
use infrarust_api::permissions::{DefaultPermissionChecker, PermissionChecker, PermissionSubject};
use infrarust_api::player::{
    BossBar, BossBarControl, BossBarHandle, BossBarUpdate, ClientSettings, ConnectionResult,
    MAX_COOKIE_SIZE, Player, ResourcePackRequest, cookie_key, session_task,
};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};
use infrarust_config::ServerAddress;
use infrarust_protocol::version::ProtocolVersion as WireVersion;

use crate::event_bus::EventBusImpl;
use crate::loadbalancer::BackendLoad;
use crate::permissions::PermissionService;

use client_state::ClientState;
use command_queue::CommandQueue;
use presentation::{BarControl, Presentation};

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
    shared: OnceLock<Weak<Self>>,
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

impl PlayerSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        protocol_version: ProtocolVersion,
        remote_addr: SocketAddr,
        current_server: Option<ServerId>,
        active: bool,
        online_mode: bool,
        command_tx: mpsc::Sender<PlayerCommand>,
        shutdown_token: CancellationToken,
        permission_checker: Arc<dyn PermissionChecker>,
        backend_load: Arc<BackendLoad>,
    ) -> Self {
        let typed_commands = CommandQueue::new(profile.username.clone());
        Self {
            player_id,
            profile,
            protocol_version,
            remote_addr,
            routing: RwLock::new(Routing {
                current: current_server,
                previous: None,
                pending: None,
            }),
            connected_address: RwLock::new(None),
            backend_load,
            connected: AtomicBool::new(true),
            active,
            online_mode,
            connected_at: SystemTime::now(),
            command_tx,
            shutdown_token,
            permission_checker: RwLock::new(permission_checker),
            permission_override: AtomicBool::new(false),
            permissions: None,
            permissions_changed: watch::Sender::new(0),
            virtual_host: None,
            released: CancellationToken::new(),
            client: ClientState::default(),
            presentation: Arc::default(),
            events: None,
            shared: OnceLock::new(),
            connects: Mutex::new(Vec::new()),
            typed_commands,
            self_waits: Mutex::default(),
        }
    }

    #[must_use]
    pub fn with_events(mut self, events: Arc<EventBusImpl>) -> Self {
        self.events = Some(events);
        self
    }

    pub fn into_shared(self) -> Arc<Self> {
        let shared = Arc::new(self);
        let _ = shared.shared.set(Arc::downgrade(&shared));
        shared
    }

    fn shared_player(&self) -> Option<Arc<dyn Player>> {
        self.shared
            .get()
            .and_then(Weak::upgrade)
            .map(|session| session as Arc<dyn Player>)
    }

    pub(crate) const fn presentation(&self) -> &Arc<Presentation> {
        &self.presentation
    }

    pub(crate) fn settle_connect(&self, target: &ServerId, result: &ConnectionResult) {
        let settled: Vec<_> = {
            let mut connects = self.connects.lock().unwrap_or_else(PoisonError::into_inner);
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

    #[must_use]
    pub fn with_permissions(mut self, permissions: Arc<PermissionService>) -> Self {
        self.permissions = Some(permissions);
        self
    }

    #[must_use]
    pub fn with_virtual_host(mut self, host: impl Into<String>) -> Self {
        self.virtual_host = Some(host.into());
        self
    }

    /// Creates a test session with a new channel and cancellation token.
    ///
    /// Returns `(session, command_rx)` so tests can inspect commands.
    pub fn new_test(active: bool) -> (Self, mpsc::Receiver<PlayerCommand>) {
        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_SIZE);
        let session = Self::new(
            PlayerId::new(1),
            GameProfile {
                uuid: Uuid::new_v4(),
                username: "TestPlayer".to_string(),
                properties: vec![],
            },
            ProtocolVersion::new(767), // 1.21
            "127.0.0.1:12345".parse().expect("valid test addr"),
            None,
            active,
            false,
            tx,
            CancellationToken::new(),
            Arc::new(DefaultPermissionChecker),
            Arc::new(BackendLoad::new()),
        );
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
        self.connects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        drop(self.typed_commands.stop());
    }

    /// Updates the current server (called by the proxy loop on server switch).
    pub fn set_current_server(&self, server: ServerId) {
        let mut routing = self.routing.write().unwrap_or_else(PoisonError::into_inner);
        if routing.current.as_ref() != Some(&server) {
            routing.previous = routing.current.replace(server);
        }
        routing.pending = None;
    }

    pub(crate) fn previous_server(&self) -> Option<ServerId> {
        self.routing
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .previous
            .clone()
    }

    pub(crate) fn set_pending_server(&self, server: ServerId) {
        self.routing
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .pending = Some(server);
    }

    pub(crate) fn counted_server(&self) -> Option<ServerId> {
        let routing = self.routing.read().unwrap_or_else(PoisonError::into_inner);
        routing.current.clone().or_else(|| routing.pending.clone())
    }

    pub fn set_connected_address(&self, address: Option<ServerAddress>) {
        let mut guard = self
            .connected_address
            .write()
            .unwrap_or_else(PoisonError::into_inner);
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
        self.connected_address
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        *self
            .permission_checker
            .write()
            .unwrap_or_else(PoisonError::into_inner) = checker;
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
        Arc::clone(
            &self
                .permission_checker
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
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
        let admitted = self
            .self_waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .admit(Instant::now());
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

    async fn approve_transfer(
        &self,
        host: String,
        port: u16,
    ) -> Result<(String, u16), PlayerError> {
        let (Some(bus), Some(player)) = (&self.events, self.shared_player()) else {
            return Ok((host, port));
        };
        let event = bus
            .fire(PreTransferEvent::new(
                player,
                host,
                port,
                TransferOrigin::Plugin,
            ))
            .await;
        match event.result() {
            PreTransferResult::Denied { reason } => {
                Err(PlayerError::Denied(Box::new(reason.clone())))
            }
            PreTransferResult::Redirect { host, port } => Ok((host.clone(), *port)),
            _ => Ok((event.host, event.port)),
        }
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

// Sealed trait implementation — allows PlayerSession to implement Player.
impl infrarust_api::player::private::Sealed for PlayerSession {}

impl Player for PlayerSession {
    fn id(&self) -> PlayerId {
        self.player_id
    }

    fn profile(&self) -> &GameProfile {
        &self.profile
    }

    fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    fn current_server(&self) -> Option<ServerId> {
        self.routing
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .current
            .clone()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(e) = self.command_tx.try_send(PlayerCommand::Kick(reason)) {
                tracing::debug!(
                    player = %self.profile.username,
                    "disconnecting without queueing the kick reason: {e}"
                );
            }
            self.shutdown_token.cancel();
        })
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendMessage(message))
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendTitle(Box::new(title)))
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendActionBar(message))
    }

    fn send_packet(&self, packet: RawPacket) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendPacket(packet))
    }

    fn switch_server(&self, target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        Box::pin(async move {
            self.send_command(PlayerCommand::SwitchServer(target, ConnectCause::Switch))
                .await
        })
    }

    fn is_online_mode(&self) -> bool {
        self.online_mode
    }

    fn has_permission(&self, permission: &str) -> bool {
        let checker = self.permission_checker();
        match &self.permissions {
            Some(permissions) => permissions.value(checker.as_ref(), permission).is_true(),
            None => checker.has_permission(permission),
        }
    }

    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some(permissions) = &self.permissions
                && !self.permission_override.load(Ordering::Acquire)
            {
                let checker = permissions.create_checker(&self.permission_subject()).await;
                self.set_permission_checker(checker);
            }
            self.permissions_changed
                .send_modify(|generation| *generation = generation.wrapping_add(1));
        })
    }

    fn connected_at(&self) -> SystemTime {
        self.connected_at
    }

    fn virtual_host(&self) -> Option<String> {
        self.virtual_host.clone()
    }

    fn client_brand(&self) -> Option<String> {
        self.client.brand()
    }

    fn settings(&self) -> Option<ClientSettings> {
        self.client.settings()
    }

    fn known_channels(&self) -> Vec<String> {
        self.client.channels()
    }

    fn ping(&self) -> Option<Duration> {
        self.client.ping()
    }

    fn send_plugin_message(&self, channel: &ChannelId, data: Bytes) -> Result<(), PlayerError> {
        self.send_message_to(MessageTarget::Client, channel, data, MAX_TO_CLIENT_PAYLOAD)
    }

    fn send_plugin_message_to_backend(
        &self,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<(), PlayerError> {
        if self.active && self.connected_address().is_none() {
            return Err(PlayerError::NoBackend);
        }
        self.send_message_to(
            MessageTarget::Backend,
            channel,
            data,
            MAX_TO_BACKEND_PAYLOAD,
        )
    }

    fn connect(&self, target: ServerId) -> BoxFuture<'_, Result<ConnectionResult, PlayerError>> {
        Box::pin(async move {
            self.ready()?;
            self.refuse_self_wait("connect")?;
            let (reply, result) = oneshot::channel();
            self.connects
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((target.clone(), reply));
            self.send_command(PlayerCommand::SwitchServer(target, ConnectCause::Switch))
                .await?;
            Ok(result.await.unwrap_or(ConnectionResult::Cancelled))
        })
    }

    fn set_player_list_header_footer(
        &self,
        header: Component,
        footer: Component,
    ) -> Result<(), PlayerError> {
        self.supports(TAB_LIST_SINCE, "tab list headers and footers")?;
        self.try_send_command(PlayerCommand::HeaderFooter(Box::new((
            header.clone(),
            footer.clone(),
        ))))?;
        self.presentation.set_header_footer(header, footer);
        Ok(())
    }

    fn clear_title(&self, reset: bool) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::ClearTitle { reset })
    }

    fn show_boss_bar(&self, bar: BossBar) -> Result<BossBarHandle, PlayerError> {
        self.supports(BOSS_BAR_SINCE, "boss bars")?;
        let id = Uuid::new_v4();
        self.presentation.show_bar(id, bar.clone());
        if let Err(e) = self.try_send_command(PlayerCommand::BossBar(
            id,
            BossBarCommand::Show(Box::new(bar)),
        )) {
            self.presentation.hide_bar(id);
            return Err(e);
        }
        let control = BarControl::new(Arc::clone(&self.presentation), self.command_tx.clone());
        Ok(BossBarHandle::new(
            id,
            Arc::new(control) as Arc<dyn BossBarControl>,
        ))
    }

    fn send_resource_pack(&self, pack: ResourcePackRequest) -> Result<(), PlayerError> {
        self.supports(RESOURCE_PACK_SINCE, "resource packs")?;
        pack.validate().map_err(PlayerError::InvalidArgument)?;
        self.try_send_command(PlayerCommand::Client(ClientCommand::PushPack(Box::new(
            pack,
        ))))
    }

    fn remove_resource_pack(&self, id: Option<Uuid>) -> Result<(), PlayerError> {
        self.supports(PACK_STACK_SINCE, "removing resource packs")?;
        self.try_send_command(PlayerCommand::Client(ClientCommand::PopPack(id)))
    }

    fn transfer(&self, host: &str, port: u16) -> BoxFuture<'_, Result<(), PlayerError>> {
        let host = host.to_string();
        Box::pin(async move {
            self.supports(TRANSFER_SINCE, "transfers")?;
            if host.is_empty() || host.chars().count() > MAX_TRANSFER_HOST {
                return Err(PlayerError::InvalidArgument(format!(
                    "a transfer host must have 1 to {MAX_TRANSFER_HOST} characters"
                )));
            }
            let (host, port) = self.approve_transfer(host, port).await?;
            self.send_command(PlayerCommand::Client(ClientCommand::Transfer {
                host,
                port,
            }))
            .await
        })
    }

    fn store_cookie(&self, key: &str, data: Bytes) -> Result<(), PlayerError> {
        self.supports(COOKIES_SINCE, "cookies")?;
        let key = cookie_key(key).map_err(PlayerError::InvalidArgument)?;
        if data.len() > MAX_COOKIE_SIZE {
            return Err(PlayerError::InvalidArgument(format!(
                "a cookie of {} bytes is over the {MAX_COOKIE_SIZE} bytes a client keeps",
                data.len()
            )));
        }
        self.try_send_command(PlayerCommand::Client(ClientCommand::StoreCookie {
            key,
            data,
        }))
    }

    fn request_cookie(&self, key: &str) -> BoxFuture<'_, Result<Option<Bytes>, PlayerError>> {
        let key = cookie_key(key);
        Box::pin(async move {
            self.supports(COOKIES_SINCE, "cookies")?;
            let key = key.map_err(PlayerError::InvalidArgument)?;
            self.refuse_self_wait("request_cookie")?;
            let (reply, answer) = oneshot::channel();
            self.send_command(PlayerCommand::Client(ClientCommand::RequestCookie {
                key,
                reply,
            }))
            .await?;
            answer.await.map_err(|_| PlayerError::Disconnected)
        })
    }
}
