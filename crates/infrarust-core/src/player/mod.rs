//! Player session management.
//!
//! Provides [`PlayerSession`] (the concrete implementation of `dyn Player`)
//! and [`PlayerCommand`] (the command channel enum for packet injection).

pub(crate) mod packets;
pub mod registry;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    SendTitle(TitleData),
    /// Display a message in the action bar.
    SendActionBar(Component),
    /// Send a raw packet to the player's client.
    SendPacket(RawPacket),
    /// Kick the player with a reason.
    Kick(Component),
    /// Switch the player to a different backend server.
    SwitchServer(ServerId),
}

/// Concrete implementation of [`Player`].
///
/// Holds identity data and a command channel to the proxy loop.
/// Sync methods (`send_message`, etc.) use `try_send` on the bounded channel.
/// Async methods (`disconnect`, `switch_server`) use `send().await`.
pub struct PlayerSession {
    player_id: PlayerId,
    profile: GameProfile,
    protocol_version: ProtocolVersion,
    remote_addr: SocketAddr,
    current_server: RwLock<Option<ServerId>>,
    connected: AtomicBool,
    active: bool,
    online_mode: bool,
    connected_at: SystemTime,
    command_tx: mpsc::Sender<PlayerCommand>,
    shutdown_token: CancellationToken,
    permission_checker: Arc<dyn PermissionChecker>,
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
    ) -> Self {
        Self {
            player_id,
            profile,
            protocol_version,
            remote_addr,
            current_server: RwLock::new(current_server),
            connected: AtomicBool::new(true),
            active,
            online_mode,
            connected_at: SystemTime::now(),
            command_tx,
            shutdown_token,
            permission_checker,
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
        );
        (session, rx)
    }

    pub fn channel() -> (mpsc::Sender<PlayerCommand>, mpsc::Receiver<PlayerCommand>) {
        mpsc::channel(COMMAND_CHANNEL_SIZE)
    }

    /// Marks the player as disconnected (called by handlers during cleanup).
    pub fn set_disconnected(&self) {
        self.connected.store(false, Ordering::Release);
    }

    /// Updates the current server (called by the proxy loop on server switch).
    pub fn set_current_server(&self, server: ServerId) {
        let mut guard = self.current_server.write().expect("lock poisoned");
        *guard = Some(server);
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown_token
    }

    pub fn game_profile(&self) -> &GameProfile {
        &self.profile
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
        self.current_server.read().expect("lock poisoned").clone()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn disconnect(&self, reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self.command_tx.send(PlayerCommand::Kick(reason)).await;
            self.shutdown_token.cancel();
        })
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendMessage(message))
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.try_send_command(PlayerCommand::SendTitle(title))
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
        self.permission_checker.permission_level()
    }

    fn has_permission(&self, permission: &str) -> bool {
        self.permission_checker.has_permission(permission)
    }

    fn connected_at(&self) -> SystemTime {
        self.connected_at
    }
}
