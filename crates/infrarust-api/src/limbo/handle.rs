use std::sync::Arc;

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
}

impl SessionHandle {
    pub fn new(inner: Arc<dyn LimboSession>) -> Self {
        Self { inner }
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
        self.inner.complete(result);
    }
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("player_id", &self.inner.player_id())
            .finish()
    }
}
