//! Ergonomic guest-side limbo-handler authoring.
//!
//! A limbo handler gates a player in a proxy-hosted waiting room (auth, queue,
//! maintenance). Implement [`LimboHandler`] and register it from
//! [`Plugin::register_limbo_handlers`](crate::Plugin::register_limbo_handlers):
//!
//! ```ignore
//! struct Gate;
//! impl LimboHandler for Gate {
//!     fn on_player_enter(&self, s: &LimboSession) -> HandlerOutcome {
//!         s.send_message(Component::text("Type /continue to proceed")).ok();
//!         HandlerOutcome::Hold
//!     }
//!     fn on_command(&self, s: &LimboSession, command: &str, _args: &[String]) {
//!         if command == "continue" { s.complete(HandlerOutcome::Accept); }
//!     }
//! }
//!
//! #[plugin]
//! impl Plugin for MyPlugin {
//!     fn register_limbo_handlers(reg: &mut LimboRegistrar) {
//!         reg.add("gate", Gate);
//!     }
//! }
//! ```

use crate::bindings::guest::{HandlerResult as WitHandlerResult, LimboSession as RawSession};
use crate::bindings::limbo::LimboEntryContext as WitEntryContext;
use crate::component::Component;

pub use crate::bindings::types::{GameProfile, PlayerError, TitleData};

pub enum HandlerOutcome {
    Accept,
    Deny(Component),
    Hold,
    Redirect(String),
    SendToLimbo(Vec<String>),
}

impl HandlerOutcome {
    pub(crate) fn into_wit(self) -> WitHandlerResult {
        match self {
            HandlerOutcome::Accept => WitHandlerResult::Accept,
            HandlerOutcome::Deny(reason) => WitHandlerResult::Deny(reason.into_json()),
            HandlerOutcome::Hold => WitHandlerResult::Hold,
            HandlerOutcome::Redirect(server) => WitHandlerResult::Redirect(server),
            HandlerOutcome::SendToLimbo(names) => WitHandlerResult::SendToLimbo(names),
        }
    }
}

pub enum EntryContext {
    InitialConnection(String),
    KickedFromServer { server: String, reason: String },
    PluginRedirect(Option<String>),
}

impl EntryContext {
    fn from_wit(c: WitEntryContext) -> Self {
        match c {
            WitEntryContext::InitialConnection(server) => EntryContext::InitialConnection(server),
            WitEntryContext::KickedFromServer((server, reason)) => {
                EntryContext::KickedFromServer { server, reason }
            }
            WitEntryContext::PluginRedirect(server) => EntryContext::PluginRedirect(server),
        }
    }
}

pub struct LimboSession<'a> {
    raw: &'a RawSession,
}

impl<'a> LimboSession<'a> {
    pub(crate) fn new(raw: &'a RawSession) -> Self {
        Self { raw }
    }

    #[must_use]
    pub fn player_id(&self) -> u64 {
        self.raw.player_id()
    }

    #[must_use]
    pub fn profile(&self) -> GameProfile {
        self.raw.profile()
    }

    #[must_use]
    pub fn entry_context(&self) -> EntryContext {
        EntryContext::from_wit(self.raw.entry_context())
    }

    /// # Errors
    /// Returns [`PlayerError`] if the message could not be delivered.
    pub fn send_message(&self, message: Component) -> Result<(), PlayerError> {
        self.raw.send_message(&message.into_json())
    }

    /// # Errors
    /// Returns [`PlayerError`] if the title could not be delivered.
    pub fn send_title(&self, title: TitleData) -> Result<(), PlayerError> {
        self.raw.send_title(&title)
    }

    /// # Errors
    /// Returns [`PlayerError`] if the message could not be delivered.
    pub fn send_action_bar(&self, message: Component) -> Result<(), PlayerError> {
        self.raw.send_action_bar(&message.into_json())
    }

    /// Releases (or redirects/denies) a held player. Call after returning
    /// [`HandlerOutcome::Hold`] from `on_player_enter`.
    pub fn complete(&self, outcome: HandlerOutcome) {
        self.raw.complete(&outcome.into_wit());
    }
}

pub trait LimboHandler {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome;

    fn on_command(&self, _session: &LimboSession, _command: &str, _args: &[String]) {}

    fn on_chat(&self, _session: &LimboSession, _message: &str) {}

    fn on_disconnect(&self, _player_id: u64) {}
}

/// Collects a plugin's limbo-handler declarations. Passed to
/// [`Plugin::register_limbo_handlers`](crate::Plugin::register_limbo_handlers).
pub struct LimboRegistrar {
    _private: (),
}

impl LimboRegistrar {
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Registers `handler` under `name` — the name a server's `limbo_handlers`
    /// config references.
    pub fn add(&mut self, name: &str, handler: impl LimboHandler + 'static) {
        crate::runtime::register_limbo_handler(name, Box::new(handler));
    }
}
