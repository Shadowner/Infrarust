use super::{GuestEvent, ResultCell};
use crate::bindings::events::{self as we, Event, EventKind, EventOutcome};
use crate::component::{Component, from_host};
use crate::types::{PlayerRef, ServerId};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ChatMessageResult {
    Allow,
    Deny(Option<Component>),
    Modify(String),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ChatMessageEvent {
    pub player: PlayerRef,
    pub message: String,
    pub signed: bool,
    pub server: Option<ServerId>,
    result: ResultCell<ChatMessageResult>,
}

impl ChatMessageEvent {
    #[must_use]
    pub const fn result(&self) -> &ChatMessageResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: ChatMessageResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(ChatMessageResult::Allow);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(ChatMessageResult::Deny(Some(reason.into())));
    }

    pub fn deny_silently(&mut self) {
        self.set_result(ChatMessageResult::Deny(None));
    }

    pub fn modify(&mut self, message: impl Into<String>) {
        self.set_result(ChatMessageResult::Modify(message.into()));
    }
}

impl GuestEvent for ChatMessageEvent {
    const KIND: EventKind = EventKind::ChatMessage;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::ChatMessage(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            message: e.message,
            signed: e.signed,
            server: e.server.map(ServerId::from),
            result: ResultCell::new(match e.result {
                we::ChatMessageResult::Allow => ChatMessageResult::Allow,
                we::ChatMessageResult::Deny(reason) => {
                    ChatMessageResult::Deny(reason.map(from_host))
                }
                we::ChatMessageResult::Modify(message) => ChatMessageResult::Modify(message),
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::ChatMessage(match r {
                    ChatMessageResult::Allow => we::ChatMessageResult::Allow,
                    ChatMessageResult::Deny(reason) => {
                        we::ChatMessageResult::Deny(reason.as_ref().map(Component::to_arena))
                    }
                    ChatMessageResult::Modify(message) => we::ChatMessageResult::Modify(message),
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::types as wt;

    fn chat(result: we::ChatMessageResult) -> ChatMessageEvent {
        ChatMessageEvent::from_event(Event::ChatMessage(we::ChatMessageEvent {
            player: wt::PlayerRef {
                id: 1,
                uuid: wt::Uuid { hi: 0, lo: 1 },
                username: "Steve".into(),
            },
            message: "hello".into(),
            signed: true,
            server: Some("lobby".into()),
            result,
        }))
        .unwrap()
    }

    #[test]
    fn a_silent_deny_carries_no_reason() {
        let mut event = chat(we::ChatMessageResult::Allow);
        assert!(event.signed);
        event.deny_silently();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::ChatMessage(we::ChatMessageResult::Deny(None))
        );
    }

    #[test]
    fn the_current_modification_is_visible() {
        let event = chat(we::ChatMessageResult::Modify("[x] hello".into()));
        assert_eq!(
            event.result(),
            &ChatMessageResult::Modify("[x] hello".into())
        );
    }
}
