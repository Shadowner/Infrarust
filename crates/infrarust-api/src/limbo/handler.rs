//! Limbo handler trait.

use crate::event::BoxFuture;
use crate::types::{Component, PlayerId, ServerId};

use super::session::LimboSession;

/// The result of a limbo handler action.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HandlerResult {
    /// Accept — continue to the next handler in the chain or the real server.
    Accept,
    /// Deny — kick the player with a reason message.
    Deny(Component),
    /// Hold — the handler will signal completion later via
    /// [`LimboSession::complete`].
    Hold,
    /// Redirect — send the player to a specific server.
    Redirect(ServerId),
}

/// A handler for a limbo stage (Tier 2).
///
/// Limbo handlers are chained in order as configured per-server. The proxy
/// handles the Minecraft protocol (JoinGame, KeepAlive, chunks) — the
/// handler only provides the game logic.
///
/// Methods use [`BoxFuture`] to allow dyn-dispatch (`Box<dyn LimboHandler>`).
/// Implement by returning `Box::pin(async move { ... })`.
///
/// # Example
/// ```ignore
/// use infrarust_api::prelude::*;
///
/// struct AuthHandler;
///
/// impl LimboHandler for AuthHandler {
///     fn name(&self) -> &str { "auth" }
///
///     fn on_player_enter(&self, session: &dyn LimboSession) -> BoxFuture<'_, HandlerResult> {
///         Box::pin(async move {
///             session.send_title(TitleData::new(
///                 Component::text("Please login").color("gold"),
///                 Component::text("/login <password>").color("gray"),
///             )).ok();
///             HandlerResult::Hold
///         })
///     }
/// }
/// ```
pub trait LimboHandler: Send + Sync {
    /// Returns the name of this handler (must match the config reference).
    fn name(&self) -> &str;

    /// Called when a player enters this limbo stage.
    ///
    /// Return [`HandlerResult::Hold`] to keep the player in limbo until
    /// [`LimboSession::complete`] is called.
    fn on_player_enter(&self, session: &dyn LimboSession) -> BoxFuture<'_, HandlerResult>;

    /// Called when the player sends a `/command args` while in this limbo stage.
    ///
    /// The default implementation does nothing.
    fn on_command(
        &self,
        _session: &dyn LimboSession,
        _command: &str,
        _args: &[&str],
    ) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// Called when the player sends a chat message (not a command).
    ///
    /// The default implementation does nothing.
    fn on_chat(&self, _session: &dyn LimboSession, _message: &str) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// Called when the player disconnects while in this limbo stage.
    ///
    /// The default implementation does nothing.
    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
