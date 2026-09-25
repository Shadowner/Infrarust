use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::command::{CommandExecuteEvent, CommandExecuteResult};

use super::{Applied, Texts, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::component;
use crate::convert;

impl WasmEvent for ChatMessageEvent {
    const KIND: EventKind = EventKind::ChatMessage;

    fn to_wit(&self) -> we::Event {
        we::Event::ChatMessage(we::ChatMessageEvent {
            player: convert::player_ref(&*self.player),
            message: self.message.clone(),
            signed: self.signed,
            server: self.server.as_ref().map(|s| s.as_str().to_owned()),
            result: match self.result() {
                ChatMessageResult::Deny { reason } => {
                    we::ChatMessageResult::Deny(reason.as_ref().map(component::to_wit))
                }
                ChatMessageResult::Modify { message } => {
                    we::ChatMessageResult::Modify(message.clone())
                }
                _ => we::ChatMessageResult::Allow,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::ChatMessage(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::ChatMessageResult::Allow => ChatMessageResult::Allow,
            we::ChatMessageResult::Deny(reason) => ChatMessageResult::Deny {
                reason: reason.as_ref().map(|reason| texts.convert(reason)),
            },
            we::ChatMessageResult::Modify(message) => ChatMessageResult::Modify { message },
        });
        texts.applied()
    }
}

impl WasmEvent for CommandExecuteEvent {
    const KIND: EventKind = EventKind::CommandExecute;

    fn to_wit(&self) -> we::Event {
        we::Event::CommandExecute(we::CommandExecuteEvent {
            player: convert::player_ref(&*self.player),
            command: self.command.clone(),
            signed: self.signed,
            server: self.server.as_ref().map(|s| s.as_str().to_owned()),
            result: match self.result() {
                CommandExecuteResult::Deny { reason } => {
                    we::CommandExecuteResult::Deny(reason.as_ref().map(component::to_wit))
                }
                CommandExecuteResult::Modify { command } => {
                    we::CommandExecuteResult::Modify(command.clone())
                }
                CommandExecuteResult::ForwardToBackend => {
                    we::CommandExecuteResult::ForwardToBackend
                }
                _ => we::CommandExecuteResult::Allow,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::CommandExecute(result) = outcome else {
            return unmatched(&outcome);
        };
        let mut texts = Texts::default();
        self.set_result(match result {
            we::CommandExecuteResult::Allow => CommandExecuteResult::Allow,
            we::CommandExecuteResult::Deny(reason) => CommandExecuteResult::Deny {
                reason: reason.as_ref().map(|reason| texts.convert(reason)),
            },
            we::CommandExecuteResult::Modify(command) => CommandExecuteResult::Modify { command },
            we::CommandExecuteResult::ForwardToBackend => CommandExecuteResult::ForwardToBackend,
        });
        texts.applied()
    }
}

#[cfg(test)]
mod tests {
    use infrarust_api::types::{Component, ServerId};

    use super::super::steve;
    use super::*;

    fn chat() -> ChatMessageEvent {
        ChatMessageEvent::new(steve(), "hello".into(), true, Some(ServerId::new("lobby")))
    }

    #[test]
    fn the_guest_sees_whether_the_message_is_signed_and_where_it_goes() {
        let we::Event::ChatMessage(record) = chat().to_wit() else {
            panic!("a chat message is sent as chat-message");
        };
        assert!(record.signed);
        assert_eq!(record.server.as_deref(), Some("lobby"));
        assert_eq!(record.result, we::ChatMessageResult::Allow);
    }

    #[test]
    fn a_command_carries_its_line_and_every_result_round_trips() {
        let mut event = CommandExecuteEvent::new(
            steve(),
            "spawn now".into(),
            false,
            Some(ServerId::new("lobby")),
        );
        let we::Event::CommandExecute(record) = event.to_wit() else {
            panic!("a command is sent as command-execute");
        };
        assert_eq!(record.command, "spawn now");
        assert_eq!(record.result, we::CommandExecuteResult::Allow);
        for (wit, native) in [
            (
                we::CommandExecuteResult::ForwardToBackend,
                CommandExecuteResult::ForwardToBackend,
            ),
            (
                we::CommandExecuteResult::Modify("hub".into()),
                CommandExecuteResult::Modify {
                    command: "hub".into(),
                },
            ),
            (
                we::CommandExecuteResult::Deny(None),
                CommandExecuteResult::Deny { reason: None },
            ),
        ] {
            assert_eq!(
                event.apply(we::EventOutcome::CommandExecute(wit.clone())),
                Applied::Set
            );
            assert_eq!(event.result(), &native);
            let we::Event::CommandExecute(record) = event.to_wit() else {
                panic!("a command is sent as command-execute");
            };
            assert_eq!(record.result, wit);
        }
    }

    #[test]
    fn a_silent_deny_carries_no_reason() {
        let mut event = chat();
        assert_eq!(
            event.apply(we::EventOutcome::ChatMessage(we::ChatMessageResult::Deny(
                None
            ))),
            Applied::Set
        );
        assert_eq!(event.result(), &ChatMessageResult::Deny { reason: None });

        event.deny(Component::text("muted"));
        let we::Event::ChatMessage(record) = event.to_wit() else {
            panic!("a chat message is sent as chat-message");
        };
        assert_eq!(
            record.result,
            we::ChatMessageResult::Deny(Some(component::to_wit(&Component::text("muted"))))
        );
    }
}
