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

use std::time::Duration;

use crate::bindings::guest::{
    HandlerResult as WitHandlerResult, LimboSession as RawSession,
    SessionEndReason as WitSessionEndReason,
};
use crate::bindings::limbo::{
    HoldTimeout as WitHoldTimeout, LimboEntryContext as WitEntryContext,
    LimboSessionHandle as RawSessionHandle, TimeoutOutcome as WitTimeoutOutcome,
};
use crate::component::Component;

pub use crate::bindings::types::{GameProfile, PlayerError, TitleData};

/// A terminal Hold outcome — what a [`HandlerOutcome::HoldWithTimeout`] resolves to
/// when its deadline elapses. Cannot itself be a Hold (the type forbids re-arming).
pub enum TimeoutOutcome {
    Accept,
    Deny(Component),
    Redirect(String),
    SendToLimbo(Vec<String>),
}

impl TimeoutOutcome {
    pub(crate) fn into_wit(self) -> WitTimeoutOutcome {
        match self {
            TimeoutOutcome::Accept => WitTimeoutOutcome::Accept,
            TimeoutOutcome::Deny(reason) => WitTimeoutOutcome::Deny(reason.into_json()),
            TimeoutOutcome::Redirect(server) => WitTimeoutOutcome::Redirect(server),
            TimeoutOutcome::SendToLimbo(names) => WitTimeoutOutcome::SendToLimbo(names),
        }
    }
}

pub enum HandlerOutcome {
    Accept,
    Deny(Component),
    Hold,
    HoldWithTimeout {
        after: Duration,
        on_timeout: TimeoutOutcome,
    },
    Redirect(String),
    SendToLimbo(Vec<String>),
}

impl HandlerOutcome {
    pub(crate) fn into_wit(self) -> WitHandlerResult {
        match self {
            HandlerOutcome::Accept => WitHandlerResult::Accept,
            HandlerOutcome::Deny(reason) => WitHandlerResult::Deny(reason.into_json()),
            HandlerOutcome::Hold => WitHandlerResult::Hold,
            HandlerOutcome::HoldWithTimeout { after, on_timeout } => {
                let after_ms = u64::try_from(after.as_millis()).unwrap_or(u64::MAX);
                WitHandlerResult::HoldWithTimeout(WitHoldTimeout {
                    after_ms,
                    on_timeout: on_timeout.into_wit(),
                })
            }
            HandlerOutcome::Redirect(server) => WitHandlerResult::Redirect(server),
            HandlerOutcome::SendToLimbo(names) => WitHandlerResult::SendToLimbo(names),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndReason {
    Disconnected,
    Released,
    Kicked,
    Redirected,
    TimedOut,
    Shutdown,
}

impl SessionEndReason {
    pub(crate) fn from_wit(r: WitSessionEndReason) -> Self {
        match r {
            WitSessionEndReason::Disconnected => SessionEndReason::Disconnected,
            WitSessionEndReason::Released => SessionEndReason::Released,
            WitSessionEndReason::Kicked => SessionEndReason::Kicked,
            WitSessionEndReason::Redirected => SessionEndReason::Redirected,
            WitSessionEndReason::TimedOut => SessionEndReason::TimedOut,
            WitSessionEndReason::Shutdown => SessionEndReason::Shutdown,
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

    #[must_use]
    pub fn handle(&self) -> SessionHandle {
        SessionHandle {
            raw: self.raw.acquire_handle(),
        }
    }
}

pub struct SessionHandle {
    raw: RawSessionHandle,
}

impl SessionHandle {
    #[must_use]
    pub fn player_id(&self) -> u64 {
        self.raw.player_id()
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

    /// Releases (or redirects/denies) the held player. A no-op if the session has
    /// already ended or advanced past the Hold this handle was minted for.
    pub fn complete(&self, outcome: HandlerOutcome) {
        self.raw.complete(&outcome.into_wit());
    }

    /// True once the session has ended (the engine cancelled it). Poll this in a
    /// repeating scheduled task to know when to stop.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.raw.cancelled()
    }
}

pub trait LimboHandler {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome;

    fn on_command(&self, _session: &LimboSession, _command: &str, _args: &[String]) {}

    fn on_chat(&self, _session: &LimboSession, _message: &str) {}

    fn on_disconnect(&self, _player_id: u64) {}

    /// Called when the player's limbo session ends, for ANY reason (released,
    /// kicked, timed out, disconnected, …). The place to drop a stored
    /// [`SessionHandle`] and cancel any scheduled tasks.
    fn on_session_end(&self, _player_id: u64, _reason: SessionEndReason) {}
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
