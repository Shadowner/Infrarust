use std::sync::Arc;

use crate::event::{Event, ResultedEvent};
use crate::player::Player;
use crate::types::{Component, GameProfile, PlayerId, ServerId};

#[non_exhaustive]
pub struct CommandExecuteEvent {
    pub player: Arc<dyn Player>,
    pub command: String,
    pub signed: bool,
    pub server: Option<ServerId>,
    result: CommandExecuteResult,
}

impl CommandExecuteEvent {
    pub fn new(
        player: Arc<dyn Player>,
        command: String,
        signed: bool,
        server: Option<ServerId>,
    ) -> Self {
        Self {
            player,
            command,
            signed,
            server,
            result: CommandExecuteResult::default(),
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn profile(&self) -> &GameProfile {
        self.player.profile()
    }

    pub fn label(&self) -> &str {
        self.command
            .split_once(char::is_whitespace)
            .map_or(self.command.as_str(), |(label, _)| label)
    }

    pub fn allow(&mut self) {
        self.result = CommandExecuteResult::Allow;
    }

    pub fn deny(&mut self, reason: Component) {
        self.result = CommandExecuteResult::Deny {
            reason: Some(reason),
        };
    }

    pub fn deny_silently(&mut self) {
        self.result = CommandExecuteResult::Deny { reason: None };
    }

    pub fn modify(&mut self, command: impl Into<String>) {
        self.result = CommandExecuteResult::Modify {
            command: command.into(),
        };
    }

    pub fn forward_to_backend(&mut self) {
        self.result = CommandExecuteResult::ForwardToBackend;
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum CommandExecuteResult {
    #[default]
    Allow,
    Deny {
        reason: Option<Component>,
    },
    Modify {
        command: String,
    },
    ForwardToBackend,
}

impl Event for CommandExecuteEvent {}
impl ResultedEvent for CommandExecuteEvent {
    type Result = CommandExecuteResult;

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
    use crate::events::chat::tests::steve;

    fn command(line: &str) -> CommandExecuteEvent {
        CommandExecuteEvent::new(steve(), line.into(), false, Some(ServerId::new("lobby")))
    }

    #[test]
    fn default_allows() {
        let event = command("spawn");
        assert_eq!(event.result(), &CommandExecuteResult::Allow);
        assert_eq!(event.player_id(), PlayerId::new(1));
        assert_eq!(event.label(), "spawn");
        assert_eq!(command("tp Steve 0 64 0").label(), "tp");
    }

    #[test]
    fn shortcuts_set_each_result() {
        let mut event = command("login hunter2");
        event.deny(Component::text("no"));
        assert_eq!(
            event.result(),
            &CommandExecuteResult::Deny {
                reason: Some(Component::text("no"))
            }
        );
        event.deny_silently();
        assert_eq!(event.result(), &CommandExecuteResult::Deny { reason: None });
        event.modify("spawn");
        assert_eq!(
            event.result(),
            &CommandExecuteResult::Modify {
                command: "spawn".into()
            }
        );
        event.forward_to_backend();
        assert_eq!(event.result(), &CommandExecuteResult::ForwardToBackend);
        event.allow();
        assert_eq!(event.result(), &CommandExecuteResult::Allow);
    }
}
