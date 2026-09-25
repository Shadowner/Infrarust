//! Player session management.
//!
//! Provides [`PlayerSession`] (the concrete implementation of `dyn Player`)
//! and [`PlayerCommand`] (the command channel enum for packet injection).

pub(crate) mod commands;
pub(crate) mod lifecycle;
pub(crate) mod packets;
pub mod registry;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::permissions::{DefaultPermissionChecker, PermissionChecker, PermissionLevel};
use infrarust_api::player::Player;
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerId, TitleData,
};
use infrarust_config::ServerAddress;

use crate::loadbalancer::BackendLoad;

/// Channel buffer size for player commands.
const COMMAND_CHANNEL_SIZE: usize = 32;

static NEXT_PLAYER_ID: AtomicU64 = AtomicU64::new(1);
pub fn next_player_id() -> PlayerId {
    PlayerId::new(NEXT_PLAYER_ID.fetch_add(1, Ordering::Relaxed))
}

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
    SwitchServer(ServerId),
}

struct Routing {
    current: Option<ServerId>,
    pending: Option<ServerId>,
}

/// Concrete implementation of [`Player`].
///
/// Holds identity data and a command channel to the proxy loop.
/// Sync methods (`send_message`, etc.) use `try_send` on the bounded channel.
/// `switch_server` uses `send().await`.
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
    released: CancellationToken,
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
        Self {
            player_id,
            profile,
            protocol_version,
            remote_addr,
            routing: RwLock::new(Routing {
                current: current_server,
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
            released: CancellationToken::new(),
        }
    }

    /// Creates a test session with a new channel and cancellation token.
    ///
    /// Returns `(session, command_rx)` so tests can inspect commands.
    pub fn new_test(active: bool) -> (Self, mpsc::Receiver<PlayerCommand>) {
        let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_SIZE);
        let session = Self::new(
            PlayerId::new(1),
            GameProfile {
                uuid: uuid::Uuid::new_v4(),
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
    }

    /// Updates the current server (called by the proxy loop on server switch).
    pub fn set_current_server(&self, server: ServerId) {
        let mut routing = self.routing.write().unwrap_or_else(PoisonError::into_inner);
        routing.current = Some(server);
        routing.pending = None;
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

    pub fn set_permission_checker(&self, checker: Arc<dyn PermissionChecker>) {
        *self
            .permission_checker
            .write()
            .unwrap_or_else(PoisonError::into_inner) = checker;
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
        if !self.active {
            return Err(PlayerError::NotActive);
        }
        if !self.connected.load(Ordering::Acquire) {
            return Err(PlayerError::Disconnected);
        }
        self.command_tx
            .try_send(cmd)
            .map_err(|e| PlayerError::SendFailed(e.to_string()))
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
            if !self.active {
                return Err(PlayerError::NotActive);
            }
            if !self.connected.load(Ordering::Acquire) {
                return Err(PlayerError::Disconnected);
            }
            self.command_tx
                .send(PlayerCommand::SwitchServer(target))
                .await
                .map_err(|e| PlayerError::SendFailed(e.to_string()))
        })
    }

    fn is_online_mode(&self) -> bool {
        self.online_mode
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission_checker().permission_level()
    }

    fn has_permission(&self, permission: &str) -> bool {
        self.permission_checker().has_permission(permission)
    }

    fn connected_at(&self) -> SystemTime {
        self.connected_at
    }
}
