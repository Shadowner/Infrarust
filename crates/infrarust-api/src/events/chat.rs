//! Chat message events.

use std::sync::Arc;

use crate::player::Player;
use crate::types::{Component, ServerId};

/// Fired when a player sends a chat message.
///
/// Listeners can allow, deny, or modify the message.
#[non_exhaustive]
pub struct ChatMessageEvent {
    pub player: Arc<dyn Player>,
    pub message: String,
    pub signed: bool,
    pub server: Option<ServerId>,
    result: ChatMessageResult,
}

impl ChatMessageEvent {
    pub fn new(
        player: Arc<dyn Player>,
        message: String,
        signed: bool,
        server: Option<ServerId>,
    ) -> Self {
        Self {
            player,
            message,
            signed,
            server,
            result: ChatMessageResult::default(),
        }
    }

    pub fn allow(&mut self) {
        self.result = ChatMessageResult::Allow;
    }

    pub fn deny(&mut self, reason: Component) {
        self.result = ChatMessageResult::Deny {
            reason: Some(reason),
        };
    }

    pub fn deny_silently(&mut self) {
        self.result = ChatMessageResult::Deny { reason: None };
    }

    pub fn modify(&mut self, message: impl Into<String>) {
        self.result = ChatMessageResult::Modify {
            message: message.into(),
        };
    }
}

/// The result of a [`ChatMessageEvent`].
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum ChatMessageResult {
    #[default]
    Allow,
    Deny {
        reason: Option<Component>,
    },
    Modify {
        message: String,
    },
}

crate::events::player_event!(ChatMessageEvent, profile);

crate::event::resulted_event!(ChatMessageEvent, ChatMessageResult);

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::event::ResultedEvent;
    use crate::test_util::MockPlayer;
    use crate::types::PlayerId;

    pub(crate) fn steve() -> Arc<dyn Player> {
        MockPlayer::new(1, "Steve").into_arc()
    }

    fn chat(message: &str) -> ChatMessageEvent {
        ChatMessageEvent::new(steve(), message.into(), false, Some(ServerId::new("lobby")))
    }

    #[test]
    fn default_allows() {
        let event = chat("hello");
        assert_eq!(event.result(), &ChatMessageResult::Allow);
        assert_eq!(event.player_id(), PlayerId::new(1));
        assert_eq!(event.profile().username, "Steve");
    }

    #[test]
    fn deny_message() {
        let mut event = chat("bad word");
        event.deny(Component::error("Watch your language!"));
        assert_eq!(
            event.result(),
            &ChatMessageResult::Deny {
                reason: Some(Component::error("Watch your language!"))
            }
        );
        event.deny_silently();
        assert_eq!(event.result(), &ChatMessageResult::Deny { reason: None });
        event.allow();
        assert_eq!(event.result(), &ChatMessageResult::Allow);
    }

    #[test]
    fn modify_message() {
        let mut event = chat("hello");
        event.modify("HELLO");
        assert_eq!(
            event.result(),
            &ChatMessageResult::Modify {
                message: "HELLO".into()
            }
        );
    }
}
