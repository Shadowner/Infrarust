use std::sync::Arc;

use crate::event::BoxFuture;
use crate::permissions::PermissionChecker;
use crate::player::Player;
use crate::types::{Component, PlayerId};

pub const CONSOLE_NAME: &str = "Console";

#[derive(Clone)]
#[non_exhaustive]
pub enum CommandSource {
    Player(Arc<dyn Player>),
    Console(Arc<dyn PermissionChecker>),
}

impl std::fmt::Debug for CommandSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Player(player) => f
                .debug_tuple("Player")
                .field(&player.profile().username)
                .finish(),
            Self::Console(_) => f.write_str("Console"),
        }
    }
}

impl CommandSource {
    pub fn console(permissions: Arc<dyn PermissionChecker>) -> Self {
        Self::Console(permissions)
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Player(player) => &player.profile().username,
            Self::Console(_) => CONSOLE_NAME,
        }
    }

    pub fn send_message(&self, message: Component) {
        match self {
            Self::Player(player) => {
                if let Err(e) = player.send_message(message) {
                    tracing::debug!(
                        player = %player.profile().username,
                        "command reply not delivered: {e}"
                    );
                }
            }
            Self::Console(_) => {
                tracing::info!(target: "infrarust::console", "{}", message.to_plain());
            }
        }
    }

    pub fn has_permission(&self, node: &str) -> bool {
        match self {
            Self::Player(player) => player.has_permission(node),
            Self::Console(permissions) => permissions.has_permission(node),
        }
    }

    pub fn player(&self) -> Option<&Arc<dyn Player>> {
        match self {
            Self::Player(player) => Some(player),
            Self::Console(_) => None,
        }
    }

    pub fn player_id(&self) -> Option<PlayerId> {
        self.player().map(|player| player.id())
    }

    pub const fn is_console(&self) -> bool {
        matches!(self, Self::Console(_))
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CommandContext {
    pub source: CommandSource,
    pub label: String,
    pub args: Vec<String>,
    pub raw_args: String,
    pub raw: String,
}

impl CommandContext {
    pub fn new(
        source: CommandSource,
        label: impl Into<String>,
        raw_args: impl Into<String>,
    ) -> Self {
        let label = label.into();
        let raw_args = raw_args.into();
        let args = raw_args.split_whitespace().map(String::from).collect();
        let raw = if raw_args.is_empty() {
            label.clone()
        } else {
            format!("{label} {raw_args}")
        };
        Self {
            source,
            label,
            args,
            raw_args,
            raw,
        }
    }

    pub fn parse(source: CommandSource, input: &str) -> Option<Self> {
        let input = input.trim();
        let (label, rest) = split_label(input);
        if label.is_empty() {
            return None;
        }
        let mut ctx = Self::new(source, label, rest.trim_start());
        input.clone_into(&mut ctx.raw);
        Some(ctx)
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SuggestContext {
    pub source: CommandSource,
    pub label: String,
    pub args: Vec<String>,
    pub raw_args: String,
}

impl SuggestContext {
    pub fn new(
        source: CommandSource,
        label: impl Into<String>,
        raw_args: impl Into<String>,
    ) -> Self {
        let raw_args = raw_args.into();
        let mut args: Vec<String> = raw_args.split_whitespace().map(String::from).collect();
        if !raw_args.is_empty() && raw_args.ends_with(char::is_whitespace) {
            args.push(String::new());
        }
        Self {
            source,
            label: label.into(),
            args,
            raw_args,
        }
    }

    pub fn partial(&self) -> &str {
        self.args.last().map_or("", String::as_str)
    }
}

pub fn split_label(input: &str) -> (&str, &str) {
    input.split_once(char::is_whitespace).unwrap_or((input, ""))
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Suggestion {
    pub text: String,
    pub tooltip: Option<Component>,
}

impl Suggestion {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tooltip: None,
        }
    }

    #[must_use]
    pub fn with_tooltip(mut self, tooltip: Component) -> Self {
        self.tooltip = Some(tooltip);
        self
    }
}

impl From<String> for Suggestion {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for Suggestion {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

pub trait CommandHandler: Send + Sync {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()>;

    fn suggest<'a>(&'a self, _ctx: SuggestContext) -> BoxFuture<'a, Vec<Suggestion>> {
        Box::pin(async { Vec::new() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub usage: Option<String>,
    pub permission: Option<String>,
    pub hidden: bool,
}

impl CommandSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            aliases: Vec::new(),
            description: String::new(),
            usage: None,
            permission: None,
            hidden: false,
        }
    }

    #[must_use]
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    #[must_use]
    pub fn aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    #[must_use]
    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = Some(usage.into());
        self
    }

    #[must_use]
    pub fn permission(mut self, node: impl Into<String>) -> Self {
        self.permission = Some(node.into());
        self
    }

    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
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
    pub fn new(
        name: impl Into<String>,
        namespaced: impl Into<String>,
        aliases: Vec<String>,
        rejected_aliases: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            namespaced: namespaced.into(),
            aliases,
            rejected_aliases,
        }
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
    pub fn new(spec: CommandSpec, plugin_id: Option<String>) -> Self {
        Self {
            name: spec.name,
            aliases: spec.aliases,
            description: spec.description,
            usage: spec.usage,
            permission: spec.permission,
            hidden: spec.hidden,
            plugin_id,
        }
    }

    pub fn namespaced(&self) -> Option<String> {
        self.plugin_id
            .as_ref()
            .map(|plugin| format!("{plugin}:{}", self.name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CommandError {
    #[error("'{0}' is reserved by the proxy")]
    Reserved(String),
    #[error("'{name}' is already registered by plugin '{plugin}'")]
    OwnedBy { name: String, plugin: String },
    #[error("'{0}' is not a valid command name")]
    InvalidName(String),
    #[error("'{0}' is not a command registered by this plugin")]
    NotOwned(String),
}

pub mod private {
    pub trait Sealed {}
}

pub trait CommandManager: Send + Sync + private::Sealed {
    fn register(
        &self,
        spec: CommandSpec,
        handler: Box<dyn CommandHandler>,
    ) -> Result<CommandRegistration, CommandError>;

    fn unregister(&self, name: &str) -> Result<(), CommandError>;

    fn list(&self) -> Vec<CommandInfo>;
}
