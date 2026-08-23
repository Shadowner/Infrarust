//! Limbo session trait.

use tokio_util::sync::CancellationToken;

use crate::error::PlayerError;
use crate::types::{Component, GameProfile, PlayerId, TitleData};

use super::context::LimboEntryContext;
use super::handle::SessionHandle;
use super::handler::HandlerResult;

pub mod private {
    /// Sealed — only the proxy implements [`LimboSession`](super::LimboSession).
    pub trait Sealed {}
}

/// A session handle for a player in a limbo stage.
///
/// Provides high-level communication methods. The proxy handles the
/// underlying Minecraft protocol (`JoinGame`, `KeepAlive`, chunks).
///
/// Obtained in [`LimboHandler`](super::handler::LimboHandler) callbacks.
pub trait LimboSession: Send + Sync + private::Sealed {
    fn player_id(&self) -> PlayerId;

    fn profile(&self) -> &GameProfile;

    fn entry_context(&self) -> &LimboEntryContext;

    /// Sends a chat message to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::SendFailed)` if the message could not be delivered.
    fn send_message(&self, message: Component) -> Result<(), PlayerError>;

    /// Sends a title display to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::SendFailed)` if the title could not be delivered.
    fn send_title(&self, title: TitleData) -> Result<(), PlayerError>;

    /// Sends an action bar message to the player.
    ///
    /// # Errors
    ///
    /// Returns `Err(PlayerError::SendFailed)` if the message could not be delivered.
    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError>;

    /// Signals that this handler is done processing the player.
    ///
    /// Call this when the handler returned [`HandlerResult::Hold`] and
    /// is now ready to release the player.
    fn complete(&self, result: HandlerResult);

    /// Like [`complete`](Self::complete) but scoped to the Hold generation captured
    /// when a [`SessionHandle`] was minted. No-ops if the session has since advanced
    /// to a later handler/Hold so a retained handle can never release the wrong
    /// Hold (e.g. a backend state-change firing after the player already moved on).
    fn complete_scoped(&self, hold_id: u64, result: HandlerResult);

    /// Returns a cloneable, `'static` handle to this session.
    ///
    /// The handle can be stored in shared state (`DashMap`, `Arc<..>`),
    /// captured in `tokio::spawn` closures, or passed to event-listener
    /// closures that outlive this callback.
    fn handle(&self) -> SessionHandle;

    /// Returns the session's cancellation token, cancelled when this limbo
    /// session ends for any reason.
    ///
    /// Handlers that spawn background tasks (timers, title animations) should
    /// `select!` on `cancellation_token().cancelled()` instead of managing their
    /// own per-session token, the engine cancels this on every exit path.
    fn cancellation_token(&self) -> CancellationToken;
}
