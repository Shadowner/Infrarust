use crate::bindings::command_manager as wcm;
use crate::bindings::guest as wg;
use crate::component::Component;
use crate::error::Error;
use crate::runtime;
use crate::types::PlayerRef;

pub const CONSOLE_NAME: &str = "Console";

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandSender {
    Console,
    Player(PlayerRef),
}

impl CommandSender {
    #[must_use]
    pub const fn player(&self) -> Option<&PlayerRef> {
        match self {
            Self::Player(player) => Some(player),
            Self::Console => None,
        }
    }

    #[must_use]
    pub const fn is_console(&self) -> bool {
        matches!(self, Self::Console)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Player(player) => &player.username,
            Self::Console => CONSOLE_NAME,
        }
    }

    pub fn send_message(&self, message: impl Into<Component>) -> Result<(), Error> {
        let message = message.into();
        match self {
            Self::Player(player) => player.handle().send_message(message),
            Self::Console => {
                crate::log::info(&message.to_plain());
                Ok(())
            }
        }
    }

    pub(crate) fn from_wit(sender: wg::CommandSender) -> Self {
        match sender {
            wg::CommandSender::Console => Self::Console,
            wg::CommandSender::Player(player) => Self::Player(PlayerRef::from_wit(player)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandInvocation {
    pub label: String,
    pub args: Vec<String>,
    pub raw: String,
    pub sender: CommandSender,
}

impl CommandInvocation {
    #[must_use]
    pub const fn player(&self) -> Option<&PlayerRef> {
        self.sender.player()
    }

    pub fn reply(&self, message: impl Into<Component>) -> Result<(), Error> {
        self.sender.send_message(message)
    }

    pub(crate) fn from_wit(invocation: wg::CommandInvocation) -> Self {
        Self {
            label: invocation.label,
            args: invocation.args,
            raw: invocation.raw,
            sender: CommandSender::from_wit(invocation.sender),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Completion {
    pub sender: CommandSender,
    pub args: Vec<String>,
    pub cursor: u32,
}

impl Completion {
    #[must_use]
    pub fn partial(&self) -> &str {
        self.args.last().map_or("", String::as_str)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Suggestion {
    pub text: String,
    pub tooltip: Option<Component>,
}

impl Suggestion {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tooltip: None,
        }
    }

    #[must_use]
    pub fn with_tooltip(mut self, tooltip: impl Into<Component>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub(crate) fn to_wit(&self) -> wg::Suggestion {
        wg::Suggestion {
            text: self.text.clone(),
            tooltip: self.tooltip.as_ref().map(Component::to_arena),
        }
    }
}

impl From<&str> for Suggestion {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for Suggestion {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandRegistration {
    pub name: String,
    pub namespaced: String,
    pub aliases: Vec<String>,
    pub rejected_aliases: Vec<String>,
}

impl CommandRegistration {
    pub(crate) fn from_wit(registration: wcm::CommandRegistration) -> Self {
        Self {
            name: registration.name,
            namespaced: registration.namespaced,
            aliases: registration.aliases,
            rejected_aliases: registration.rejected_aliases,
        }
    }

    pub fn unregister(self) -> Result<bool, Error> {
        runtime::unregister_command(&self.name)
    }
}

pub(crate) type CommandClosure = Box<dyn FnMut(CommandInvocation)>;
pub(crate) type CompletionClosure = Box<dyn Fn(&Completion) -> Vec<Suggestion>>;

#[must_use = "a command is only registered once `register` is called"]
pub struct CommandBuilder {
    spec: wcm::CommandSpec,
    handler: Option<CommandClosure>,
    completer: Option<CompletionClosure>,
}

impl CommandBuilder {
    pub(crate) fn new(name: String) -> Self {
        Self {
            spec: wcm::CommandSpec {
                name,
                aliases: Vec::new(),
                description: String::new(),
                usage: None,
                permission: None,
                hidden: false,
            },
            handler: None,
            completer: None,
        }
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.spec.aliases.push(alias.into());
        self
    }

    pub fn aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.spec
            .aliases
            .extend(aliases.into_iter().map(Into::into));
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.spec.description = description.into();
        self
    }

    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.spec.usage = Some(usage.into());
        self
    }

    pub fn permission(mut self, node: impl Into<String>) -> Self {
        self.spec.permission = Some(node.into());
        self
    }

    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.spec.hidden = hidden;
        self
    }

    pub fn handler(mut self, handler: impl FnMut(CommandInvocation) + 'static) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }

    pub fn completer<S: Into<Suggestion>>(
        mut self,
        completer: impl Fn(&Completion) -> Vec<S> + 'static,
    ) -> Self {
        self.completer = Some(Box::new(move |completion| {
            completer(completion).into_iter().map(Into::into).collect()
        }));
        self
    }

    pub fn register(self) -> Result<CommandRegistration, Error> {
        let handler = self.handler.unwrap_or_else(|| Box::new(|_| {}));
        runtime::register_command(self.spec, handler, self.completer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completion_exposes_the_partial_argument() {
        let completion = Completion {
            sender: CommandSender::Console,
            args: vec!["a".into(), "wo".into()],
            cursor: 4,
        };
        assert_eq!(completion.partial(), "wo");
        assert_eq!(completion.sender.name(), CONSOLE_NAME);
    }

    #[test]
    fn a_suggestion_tooltip_crosses_as_an_arena() {
        let wire = Suggestion::from("world")
            .with_tooltip(Component::text("the world").italic())
            .to_wit();
        assert_eq!(wire.text, "world");
        assert_eq!(
            Component::from_arena(wire.tooltip.unwrap()).unwrap(),
            Component::text("the world").italic()
        );
    }

    #[test]
    fn an_invocation_names_its_player() {
        let invocation = CommandInvocation::from_wit(wg::CommandInvocation {
            label: "greet".into(),
            args: vec!["world".into()],
            raw: "greet world".into(),
            sender: wg::CommandSender::Player(crate::bindings::types::PlayerRef {
                id: 4,
                uuid: crate::bindings::types::Uuid { hi: 0, lo: 4 },
                username: "Alex".into(),
            }),
        });
        assert_eq!(invocation.player().map(|p| p.id.as_u64()), Some(4));
        assert_eq!(invocation.sender.name(), "Alex");
    }
}
