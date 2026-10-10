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

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub usage: Option<String>,
    pub permission: Option<String>,
    pub hidden: bool,
    pub plugin_id: Option<String>,
}

impl CommandInfo {
    #[must_use]
    pub fn namespaced(&self) -> Option<String> {
        self.plugin_id
            .as_ref()
            .map(|plugin| format!("{plugin}:{}", self.name))
    }

    fn from_wit(info: wcm::CommandInfo) -> Self {
        let spec = info.spec;
        Self {
            name: spec.name,
            aliases: spec.aliases,
            description: spec.description,
            usage: spec.usage,
            permission: spec.permission,
            hidden: spec.hidden,
            plugin_id: info.plugin_id,
        }
    }

    fn from_wit_list(infos: Vec<wcm::CommandInfo>) -> Vec<Self> {
        infos.into_iter().map(Self::from_wit).collect()
    }
}

pub struct Commands;

impl Commands {
    pub fn get(label: &str) -> Result<Option<CommandInfo>, Error> {
        Ok(crate::host::get_command(label)?.map(CommandInfo::from_wit))
    }

    pub fn get_by_name(name: &str) -> Result<Option<CommandInfo>, Error> {
        Ok(crate::host::get_command_by_name(name)?.map(CommandInfo::from_wit))
    }

    pub fn get_by_alias(alias: &str) -> Result<Option<CommandInfo>, Error> {
        Ok(crate::host::get_command_by_alias(alias)?.map(CommandInfo::from_wit))
    }

    pub fn contains(label: &str) -> Result<bool, Error> {
        Ok(crate::host::contains_command(label)?)
    }

    pub fn list() -> Result<Vec<CommandInfo>, Error> {
        Ok(CommandInfo::from_wit_list(crate::host::list_commands()?))
    }

    pub fn list_owned() -> Result<Vec<CommandInfo>, Error> {
        Ok(CommandInfo::from_wit_list(
            crate::host::list_owned_commands()?,
        ))
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::context::Context;
    use crate::error::ErrorKind;
    use crate::host;

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

    fn host_command(name: &str, aliases: &[&str], plugin_id: Option<&str>) -> wcm::CommandInfo {
        wcm::CommandInfo {
            spec: wcm::CommandSpec {
                name: name.to_owned(),
                aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
                description: format!("{name} help"),
                usage: Some(format!("/{name}")),
                permission: Some(format!("{name}.use")),
                hidden: true,
            },
            plugin_id: plugin_id.map(str::to_owned),
        }
    }

    fn labels(infos: &[CommandInfo]) -> Vec<String> {
        infos
            .iter()
            .map(|info| info.namespaced().unwrap_or_else(|| info.name.clone()))
            .collect()
    }

    fn is_refused<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
        matches!(result, Err(error) if error.kind() == ErrorKind::Conflict
            && error.message() == "command-lookup is refused")
    }

    #[test]
    fn commands_are_looked_up_by_any_label_or_only_by_their_name_or_alias() {
        host::with_fake(|h| {
            h.command_table
                .push(host_command("infrarust", &["ir"], None));
            h.command_table
                .push(host_command("warp", &["w"], Some("other")));
        });
        Context::new()
            .command("Home")
            .alias("H")
            .register()
            .unwrap();

        let home = Commands::get("h").unwrap().expect("an alias reaches home");
        assert_eq!(home.name, "home");
        assert_eq!(home.aliases, ["h"]);
        assert_eq!(home.namespaced().as_deref(), Some("fake:home"));
        for label in ["HOME", "fake:home"] {
            assert_eq!(Commands::get(label), Ok(Some(home.clone())), "{label}");
        }
        assert_eq!(Commands::get_by_name("Fake:Home"), Ok(Some(home.clone())));
        assert_eq!(Commands::get_by_name("h"), Ok(None));
        assert_eq!(Commands::get_by_alias("H"), Ok(Some(home.clone())));
        assert_eq!(Commands::get_by_alias("home"), Ok(None));

        let builtin = Commands::get_by_alias("ir")
            .unwrap()
            .expect("a built-in alias resolves");
        assert_eq!(builtin.name, "infrarust");
        assert_eq!(builtin.plugin_id, None);
        assert_eq!(builtin.namespaced(), None);
        assert_eq!(
            Commands::get("other:warp"),
            Ok(Some(CommandInfo {
                name: "warp".to_owned(),
                aliases: vec!["w".to_owned()],
                description: "warp help".to_owned(),
                usage: Some("/warp".to_owned()),
                permission: Some("warp.use".to_owned()),
                hidden: true,
                plugin_id: Some("other".to_owned()),
            }))
        );
        assert_eq!(Commands::contains("W"), Ok(true));
        assert_eq!(Commands::contains("nope"), Ok(false));
        assert_eq!(Commands::get("nope"), Ok(None));

        assert_eq!(
            labels(&Commands::list().unwrap()),
            ["fake:home", "infrarust", "other:warp"]
        );
        assert_eq!(labels(&Commands::list_owned().unwrap()), ["fake:home"]);

        assert_eq!(Context::new().unregister_command("home"), Ok(true));
        assert_eq!(Commands::list_owned(), Ok(vec![]));
        assert_eq!(Commands::contains("h"), Ok(false));
        assert_eq!(
            labels(&Commands::list().unwrap()),
            ["infrarust", "other:warp"]
        );
    }

    #[test]
    fn a_refused_lookup_is_an_error_not_an_empty_answer() {
        host::with_fake(|h| {
            h.command_table
                .push(host_command("infrarust", &["ir"], None));
            h.refused.insert("command-lookup".to_owned());
        });
        assert!(is_refused(&Commands::get("infrarust")));
        assert!(is_refused(&Commands::get_by_name("infrarust")));
        assert!(is_refused(&Commands::get_by_alias("ir")));
        assert!(is_refused(&Commands::contains("infrarust")));
        assert!(is_refused(&Commands::list()));
        assert!(is_refused(&Commands::list_owned()));
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
