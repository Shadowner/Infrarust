use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::error::PlayerError;
use crate::limbo::context::LimboEntryContext;
use crate::limbo::handle::SessionHandle;
use crate::limbo::handler::HandlerResult;
use crate::limbo::session::LimboSession;
use crate::limbo::session::private::Sealed;
use crate::types::{Component, GameProfile, PlayerId, TitleData};

use super::lock;

#[derive(Default)]
struct Recorded {
    messages: Mutex<Vec<Component>>,
    titles: Mutex<Vec<TitleData>>,
    action_bars: Mutex<Vec<Component>>,
    completions: Mutex<Vec<HandlerResult>>,
}

pub struct RecordingLimboSession {
    player_id: PlayerId,
    profile: GameProfile,
    entry_context: LimboEntryContext,
    recorded: Arc<Recorded>,
    cancel: CancellationToken,
}

impl RecordingLimboSession {
    #[must_use]
    pub fn new(
        player_id: PlayerId,
        profile: GameProfile,
        entry_context: LimboEntryContext,
    ) -> Arc<Self> {
        Arc::new(Self {
            player_id,
            profile,
            entry_context,
            recorded: Arc::new(Recorded::default()),
            cancel: CancellationToken::new(),
        })
    }

    #[must_use]
    pub fn messages(&self) -> Vec<Component> {
        lock(&self.recorded.messages).clone()
    }

    #[must_use]
    pub fn titles(&self) -> Vec<TitleData> {
        lock(&self.recorded.titles).clone()
    }

    #[must_use]
    pub fn action_bars(&self) -> Vec<Component> {
        lock(&self.recorded.action_bars).clone()
    }

    #[must_use]
    pub fn completions(&self) -> Vec<HandlerResult> {
        lock(&self.recorded.completions).clone()
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
        lock(&self.recorded.messages).push(message);
        Ok(())
    }

    fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        lock(&self.recorded.titles).push(title);
        Ok(())
    }

    fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        lock(&self.recorded.action_bars).push(message);
        Ok(())
    }

    fn complete(&self, result: HandlerResult) {
        lock(&self.recorded.completions).push(result);
    }

    fn complete_scoped(&self, _hold_id: u64, result: HandlerResult) {
        lock(&self.recorded.completions).push(result);
    }

    fn handle(&self) -> SessionHandle {
        let twin = Arc::new(Self {
            player_id: self.player_id,
            profile: self.profile.clone(),
            entry_context: self.entry_context.clone(),
            recorded: Arc::clone(&self.recorded),
            cancel: self.cancel.clone(),
        });
        SessionHandle::new(twin, 0)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ServerId;

    #[test]
    fn a_handle_records_into_the_same_session() {
        let session = RecordingLimboSession::new(
            PlayerId::new(1),
            GameProfile {
                uuid: uuid::Uuid::nil(),
                username: "Steve".to_string(),
                properties: Vec::new(),
            },
            LimboEntryContext::InitialConnection {
                target_server: ServerId::new("hub"),
            },
        );
        let handle = session.handle();
        handle.send_message(Component::text("hi")).unwrap();
        handle.complete(HandlerResult::Accept);
        assert_eq!(session.messages(), vec![Component::text("hi")]);
        assert!(matches!(
            session.completions().as_slice(),
            [HandlerResult::Accept]
        ));
        assert_eq!(handle.player_id(), PlayerId::new(1));
    }
}
