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

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum CommandExecuteResult {
    Allow,
    Deny(Option<Component>),
    Modify(String),
    ForwardToBackend,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CommandExecuteEvent {
    pub player: PlayerRef,
    pub command: String,
    pub signed: bool,
    pub server: Option<ServerId>,
    result: ResultCell<CommandExecuteResult>,
}

impl CommandExecuteEvent {
    #[must_use]
    pub fn label(&self) -> &str {
        self.command
            .split_once(char::is_whitespace)
            .map_or(self.command.as_str(), |(label, _)| label)
    }

    #[must_use]
    pub const fn result(&self) -> &CommandExecuteResult {
        self.result.get()
    }

    pub fn set_result(&mut self, result: CommandExecuteResult) {
        self.result.set(result);
    }

    pub fn allow(&mut self) {
        self.set_result(CommandExecuteResult::Allow);
    }

    pub fn deny(&mut self, reason: impl Into<Component>) {
        self.set_result(CommandExecuteResult::Deny(Some(reason.into())));
    }

    pub fn deny_silently(&mut self) {
        self.set_result(CommandExecuteResult::Deny(None));
    }

    pub fn modify(&mut self, command: impl Into<String>) {
        self.set_result(CommandExecuteResult::Modify(command.into()));
    }

    pub fn forward_to_backend(&mut self) {
        self.set_result(CommandExecuteResult::ForwardToBackend);
    }
}

impl GuestEvent for CommandExecuteEvent {
    const KIND: EventKind = EventKind::CommandExecute;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::CommandExecute(e) = ev else {
            return None;
        };
        Some(Self {
            player: PlayerRef::from_wit(e.player),
            command: e.command,
            signed: e.signed,
            server: e.server.map(ServerId::from),
            result: ResultCell::new(match e.result {
                we::CommandExecuteResult::Allow => CommandExecuteResult::Allow,
                we::CommandExecuteResult::Deny(reason) => {
                    CommandExecuteResult::Deny(reason.map(from_host))
                }
                we::CommandExecuteResult::Modify(command) => CommandExecuteResult::Modify(command),
                we::CommandExecuteResult::ForwardToBackend => {
                    CommandExecuteResult::ForwardToBackend
                }
            }),
        })
    }

    fn into_outcome(self) -> EventOutcome {
        self.result
            .into_changed()
            .map_or(EventOutcome::Unchanged, |r| {
                EventOutcome::CommandExecute(match r {
                    CommandExecuteResult::Allow => we::CommandExecuteResult::Allow,
                    CommandExecuteResult::Deny(reason) => {
                        we::CommandExecuteResult::Deny(reason.as_ref().map(Component::to_arena))
                    }
                    CommandExecuteResult::Modify(command) => {
                        we::CommandExecuteResult::Modify(command)
                    }
                    CommandExecuteResult::ForwardToBackend => {
                        we::CommandExecuteResult::ForwardToBackend
                    }
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

    #[test]
    fn a_command_can_be_forwarded_and_its_label_is_the_first_word() {
        let mut event =
            CommandExecuteEvent::from_event(Event::CommandExecute(we::CommandExecuteEvent {
                player: wt::PlayerRef {
                    id: 1,
                    uuid: wt::Uuid { hi: 0, lo: 1 },
                    username: "Steve".into(),
                },
                command: "tp Steve 0 64 0".into(),
                signed: false,
                server: None,
                result: we::CommandExecuteResult::Allow,
            }))
            .unwrap();
        assert_eq!(event.label(), "tp");
        event.forward_to_backend();
        assert_eq!(
            event.into_outcome(),
            EventOutcome::CommandExecute(we::CommandExecuteResult::ForwardToBackend)
        );
    }
}
