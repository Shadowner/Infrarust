//! The `Context` handed to `on_enable`/`on_disable`.

use std::time::Duration;

use crate::event::{EventPriority, GuestEvent};
use crate::runtime;
use crate::services;

/// What a command handler receives when its command is invoked.
pub struct CommandInvocation {
    pub args: Vec<String>,
    pub player: Option<u64>,
}

/// A scheduled-task handle, usable with [`Context::cancel`].
pub type TaskHandle = u64;

pub struct EventSubscription {
    id: u64,
}

impl EventSubscription {
    pub fn cancel(self) {
        runtime::unsubscribe_event(self.id);
    }
}

/// The plugin's entry point into the host: event subscription, command and task
/// registration, and service accessors.
pub struct Context {
    _priv: (),
}

impl Context {
    pub(crate) fn new() -> Self {
        Self { _priv: () }
    }

    /// Subscribe a handler for event `E`, returning a handle to this one
    /// subscription. Multiple handlers may share a kind and fire in priority
    /// order. Drop the handle to keep the subscription, or call
    /// [`cancel`](EventSubscription::cancel) to remove just this handler.
    pub fn on<E: GuestEvent>(
        &self,
        priority: EventPriority,
        handler: impl FnMut(&mut E) + 'static,
    ) -> EventSubscription {
        EventSubscription {
            id: runtime::register_event::<E>(priority, handler),
        }
    }

    /// Register a command, returning a builder for aliases/description.
    pub fn command<'a>(
        &self,
        name: &'a str,
        handler: impl FnMut(CommandInvocation) + 'static,
    ) -> CommandBuilder<'a> {
        CommandBuilder {
            name,
            aliases: Vec::new(),
            description: String::new(),
            handler: Box::new(handler),
            completer: None,
        }
    }

    /// Run `task` once after `after`. Returns a handle for [`cancel`](Self::cancel).
    pub fn delay(&self, after: Duration, task: impl FnMut() + 'static) -> TaskHandle {
        runtime::schedule_delay(millis(after), Box::new(task))
    }

    /// Run `task` every `period`. Returns a handle for [`cancel`](Self::cancel).
    pub fn interval(&self, period: Duration, task: impl FnMut() + 'static) -> TaskHandle {
        runtime::schedule_interval(millis(period), Box::new(task))
    }

    pub fn cancel(&self, handle: TaskHandle) {
        crate::bindings::scheduler::cancel(handle);
    }

    #[must_use]
    pub fn player_registry(&self) -> services::Players {
        services::Players
    }

    #[must_use]
    pub fn server_manager(&self) -> services::Servers {
        services::Servers
    }

    #[must_use]
    pub fn ban_service(&self) -> services::Bans {
        services::Bans
    }

    #[must_use]
    pub fn config_service(&self) -> services::Config {
        services::Config
    }
}

type CompletionFn = Box<dyn Fn(&[String], u32) -> Vec<String>>;

/// Fluent builder returned by [`Context::command`]; call [`register`](Self::register) to finish.
pub struct CommandBuilder<'a> {
    name: &'a str,
    aliases: Vec<String>,
    description: String,
    handler: Box<dyn FnMut(CommandInvocation)>,
    completer: Option<CompletionFn>,
}

impl CommandBuilder<'_> {
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
    pub fn completer(
        mut self,
        completer: impl Fn(&[String], u32) -> Vec<String> + 'static,
    ) -> Self {
        self.completer = Some(Box::new(completer));
        self
    }

    pub fn register(self) {
        runtime::register_command(
            self.name,
            &self.aliases,
            &self.description,
            self.handler,
            self.completer,
        );
    }
}

#[allow(clippy::cast_possible_truncation)]
fn millis(d: Duration) -> u64 {
    d.as_millis().min(u128::from(u64::MAX)) as u64
}
