use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::error::PlayerError;
use crate::types::{Component, PlayerId, TitleData};

use super::handler::HandlerResult;
use super::session::LimboSession;

/// A cloneable, `'static` handle to a limbo session.
///
/// Unlike `&dyn LimboSession` (lifetime-bound), a `SessionHandle` can be
/// stored in shared state, captured in spawned tasks, or passed to callbacks.
///
/// Obtain one via [`LimboSession::handle()`].
#[derive(Clone)]
pub struct SessionHandle {
    inner: Arc<dyn LimboSession>,
    /// The Hold generation captured when this handle was minted. Completion is
    /// scoped to it so a stale handle can't release a later Hold.
    hold_id: u64,
}

impl SessionHandle {
    pub fn new(inner: Arc<dyn LimboSession>, hold_id: u64) -> Self {
        Self { inner, hold_id }
    }

    pub fn player_id(&self) -> PlayerId {
        self.inner.player_id()
    }

    pub fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.inner.send_message(message)
    }

    pub fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.inner.send_title(title)
    }

    pub fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.inner.send_action_bar(message)
    }

    pub fn complete(&self, result: HandlerResult) {
        self.inner.complete_scoped(self.hold_id, result);
    }

    /// The session's cancellation token, cancelled when the limbo session ends.
    ///
    /// Store-and-`select!` on `cancellation_token().cancelled()` to stop background
    /// tasks on any session exit, without managing a separate token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancellation_token()
    }

    pub fn as_session(&self) -> Arc<dyn LimboSession> {
        Arc::clone(&self.inner)
    }
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("player_id", &self.inner.player_id())
            .finish()
    }
}
