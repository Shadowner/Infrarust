//! Chat message events.

use crate::event::{Event, ResultedEvent};
use crate::types::{Component, PlayerId};

/// Fired when a player sends a chat message.
///
/// Listeners can allow, deny, or modify the message.
pub struct ChatMessageEvent {
    /// The player who sent the message.
    pub player_id: PlayerId,
    /// The original message text.
    pub message: String,
    result: ChatMessageResult,
}

impl ChatMessageEvent {
    /// Creates a new `ChatMessageEvent`.
    pub fn new(player_id: PlayerId, message: String) -> Self {
        Self {
            player_id,
            message,
            result: ChatMessageResult::default(),
        }
    }

    /// Shortcut: deny the message with a reason.
    pub fn deny(&mut self, reason: Component) {
        self.result = ChatMessageResult::Deny { reason };
    }

    /// Shortcut: modify the message text.
    pub fn modify(&mut self, new_message: String) {
        self.result = ChatMessageResult::Modify { new_message };
    }
}

/// The result of a [`ChatMessageEvent`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum ChatMessageResult {
    /// Allow the message through unmodified.
    #[default]
    Allow,
    /// Block the message and optionally notify the sender.
    Deny {
        /// The reason shown to the sender.
        reason: Component,
    },
    /// Replace the message content.
    Modify {
        /// The new message text.
        new_message: String,
    },
}

impl Event for ChatMessageEvent {}
impl ResultedEvent for ChatMessageEvent {
    type Result = ChatMessageResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allows() {
        let event = ChatMessageEvent::new(PlayerId::new(1), "hello".into());
        assert!(matches!(event.result(), ChatMessageResult::Allow));
    }

    #[test]
    fn deny_message() {
        let mut event = ChatMessageEvent::new(PlayerId::new(1), "bad word".into());
        event.deny(Component::error("Watch your language!"));
        assert!(matches!(event.result(), ChatMessageResult::Deny { .. }));
    }

    #[test]
    fn modify_message() {
        let mut event = ChatMessageEvent::new(PlayerId::new(1), "hello".into());
        event.modify("HELLO".into());
        match event.result() {
            ChatMessageResult::Modify { new_message } => assert_eq!(new_message, "HELLO"),
            _ => panic!("expected Modify"),
        }
    }
}
