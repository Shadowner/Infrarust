use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio_util::sync::CancellationToken;

use crate::error::PlayerError;
use crate::types::{Component, GameProfile, PlayerId, TitleData};

use super::context::LimboEntryContext;
use super::handle::SessionHandle;
use super::handler::HandlerResult;
use super::session::private::Sealed;
use super::session::LimboSession;

pub struct RecordingLimboSession {
    player_id: PlayerId,
    profile: GameProfile,
    entry_context: LimboEntryContext,
    messages: Mutex<Vec<Component>>,
    titles: Mutex<Vec<TitleData>>,
    action_bars: Mutex<Vec<Component>>,
    completions: Mutex<Vec<HandlerResult>>,
    cancel: CancellationToken,
    self_ref: OnceLock<Weak<Self>>,
}

impl RecordingLimboSession {
    #[must_use]
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        entry_context: LimboEntryContext,
    ) -> Arc<Self> {
        let session = Arc::new(Self {
            player_id,
            profile,
            entry_context,
            messages: Mutex::new(Vec::new()),
            titles: Mutex::new(Vec::new()),
            action_bars: Mutex::new(Vec::new()),
            completions: Mutex::new(Vec::new()),
            cancel: CancellationToken::new(),
            self_ref: OnceLock::new(),
        });
        let _ = session.self_ref.set(Arc::downgrade(&session));
        session
    }

    #[must_use]
    pub fn messages(&self) -> Vec<Component> {
        self.messages.lock().expect("lock poisoned").clone()
    }

    #[must_use]
    pub fn titles(&self) -> Vec<TitleData> {
        self.titles.lock().expect("lock poisoned").clone()
    }

    #[must_use]
    pub fn action_bars(&self) -> Vec<Component> {
        self.action_bars.lock().expect("lock poisoned").clone()
    }

    #[must_use]
    pub fn completions(&self) -> Vec<HandlerResult> {
        self.completions.lock().expect("lock poisoned").clone()
    }
}

impl Sealed for RecordingLimboSession {}

impl LimboSession for RecordingLimboSession {
    fn player_id(&self) -> PlayerId {
        self.player_id
    }

    fn profile(&self) -> &GameProfile {
        &self.profile
    }

    fn entry_context(&self) -> &LimboEntryContext {
        &self.entry_context
    }

    fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.messages.lock().expect("lock poisoned").push(message);
        Ok(())
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.titles.lock().expect("lock poisoned").push(title);
        Ok(())
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.action_bars
            .lock()
            .expect("lock poisoned")
            .push(message);
        Ok(())
    }

    fn complete(&self, result: HandlerResult) {
        self.completions
            .lock()
            .expect("lock poisoned")
            .push(result);
    }

    fn complete_scoped(&self, _hold_id: u64, result: HandlerResult) {
        self.completions
            .lock()
            .expect("lock poisoned")
            .push(result);
    }

    fn handle(&self) -> SessionHandle {
        let arc = self
            .self_ref
            .get()
            .and_then(Weak::upgrade)
            .expect("RecordingLimboSession must be constructed via new()");
        SessionHandle::new(arc, 0)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}
