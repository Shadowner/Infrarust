use std::time::Duration;

use crate::bindings::guest::{
    HandlerResult as WitHandlerResult, LimboSession as RawSession,
    SessionEndReason as WitSessionEndReason,
};
use crate::bindings::limbo::{
    HoldTimeout as WitHoldTimeout, LimboEntryContext as WitEntryContext,
    LimboSessionHandle as RawSessionHandle, TimeoutOutcome as WitTimeoutOutcome,
};
use crate::component::{Component, from_host};
use crate::error::Error;
use crate::player::TitleData;
use crate::types::{GameProfile, PlayerId, ServerId, millis};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TimeoutOutcome {
    Accept,
    Deny(Component),
    Redirect(ServerId),
    SendToLimbo(Vec<String>),
}

impl TimeoutOutcome {
    pub(crate) fn into_wit(self) -> WitTimeoutOutcome {
        match self {
            Self::Accept => WitTimeoutOutcome::Accept,
            Self::Deny(reason) => WitTimeoutOutcome::Deny(reason.to_arena()),
            Self::Redirect(server) => WitTimeoutOutcome::Redirect(server.into_string()),
            Self::SendToLimbo(names) => WitTimeoutOutcome::SendToLimbo(names),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HandlerOutcome {
    Accept,
    Deny(Component),
    Hold,
    HoldWithTimeout {
        after: Duration,
        on_timeout: TimeoutOutcome,
    },
    Redirect(ServerId),
    SendToLimbo(Vec<String>),
}

impl HandlerOutcome {
    pub(crate) fn into_wit(self) -> WitHandlerResult {
        match self {
            Self::Accept => WitHandlerResult::Accept,
            Self::Deny(reason) => WitHandlerResult::Deny(reason.to_arena()),
            Self::Hold => WitHandlerResult::Hold,
            Self::HoldWithTimeout { after, on_timeout } => {
                WitHandlerResult::HoldWithTimeout(WitHoldTimeout {
                    after_ms: millis(after),
                    on_timeout: on_timeout.into_wit(),
                })
            }
            Self::Redirect(server) => WitHandlerResult::Redirect(server.into_string()),
            Self::SendToLimbo(names) => WitHandlerResult::SendToLimbo(names),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionEndReason {
    Disconnected,
    Released,
    Kicked,
    Redirected,
    TimedOut,
    Shutdown,
}

impl SessionEndReason {
    pub(crate) const fn from_wit(r: WitSessionEndReason) -> Self {
        match r {
            WitSessionEndReason::Disconnected => Self::Disconnected,
            WitSessionEndReason::Released => Self::Released,
            WitSessionEndReason::Kicked => Self::Kicked,
            WitSessionEndReason::Redirected => Self::Redirected,
            WitSessionEndReason::TimedOut => Self::TimedOut,
            WitSessionEndReason::Shutdown => Self::Shutdown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum EntryContext {
    InitialConnection(ServerId),
    KickedFromServer { server: ServerId, reason: Component },
    PluginRedirect(Option<ServerId>),
}

impl EntryContext {
    pub(crate) fn from_wit(c: WitEntryContext) -> Self {
        match c {
            WitEntryContext::InitialConnection(server) => {
                Self::InitialConnection(ServerId::from(server))
            }
            WitEntryContext::KickedFromServer((server, reason)) => Self::KickedFromServer {
                server: ServerId::from(server),
                reason: from_host(reason),
            },
            WitEntryContext::PluginRedirect(server) => {
                Self::PluginRedirect(server.map(ServerId::from))
            }
        }
    }
}

pub struct LimboSession<'a> {
    raw: &'a RawSession,
}

impl<'a> LimboSession<'a> {
    pub(crate) const fn new(raw: &'a RawSession) -> Self {
        Self { raw }
    }

    #[must_use]
    pub fn player_id(&self) -> PlayerId {
        PlayerId::new(self.raw.player_id())
    }

    #[must_use]
    pub fn profile(&self) -> GameProfile {
        GameProfile::from_wit(self.raw.profile())
    }

    #[must_use]
    pub fn entry_context(&self) -> EntryContext {
        EntryContext::from_wit(self.raw.entry_context())
    }

    pub fn send_message(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(self.raw.send_message(&message.into().to_arena())?)
    }

    pub fn send_title(&self, title: &TitleData) -> Result<(), Error> {
        Ok(self.raw.send_title(&title.to_wit())?)
    }

    pub fn send_action_bar(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(self.raw.send_action_bar(&message.into().to_arena())?)
    }

    pub fn complete(&self, outcome: HandlerOutcome) -> Result<(), Error> {
        Ok(self.raw.complete(&outcome.into_wit())?)
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
    pub fn player_id(&self) -> PlayerId {
        PlayerId::new(self.raw.player_id())
    }

    pub fn send_message(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(self.raw.send_message(&message.into().to_arena())?)
    }

    pub fn send_title(&self, title: &TitleData) -> Result<(), Error> {
        Ok(self.raw.send_title(&title.to_wit())?)
    }

    pub fn send_action_bar(&self, message: impl Into<Component>) -> Result<(), Error> {
        Ok(self.raw.send_action_bar(&message.into().to_arena())?)
    }

    pub fn complete(&self, outcome: HandlerOutcome) -> Result<(), Error> {
        Ok(self.raw.complete(&outcome.into_wit())?)
    }

    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.raw.cancelled()
    }
}

pub trait LimboHandler {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome;

    fn on_command(&self, _session: &LimboSession, _command: &str, _args: &[String]) {}

    fn on_chat(&self, _session: &LimboSession, _message: &str) {}

    fn on_disconnect(&self, _player: PlayerId) {}

    fn on_session_end(&self, _player: PlayerId, _reason: SessionEndReason) {}
}

pub struct LimboRegistrar {
    _private: (),
}

impl LimboRegistrar {
    pub(crate) const fn new() -> Self {
        Self { _private: () }
    }

    pub fn add(&mut self, name: &str, handler: impl LimboHandler + 'static) {
        crate::runtime::register_limbo_handler(name, Box::new(handler));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_with_timeout_crosses_as_milliseconds() {
        let outcome = HandlerOutcome::HoldWithTimeout {
            after: Duration::from_secs(5),
            on_timeout: TimeoutOutcome::Deny(Component::text("Timed out")),
        };
        let WitHandlerResult::HoldWithTimeout(hold) = outcome.into_wit() else {
            panic!("a timed hold stays a timed hold");
        };
        assert_eq!(hold.after_ms, 5_000);
        assert_eq!(
            hold.on_timeout,
            WitTimeoutOutcome::Deny(Component::text("Timed out").to_arena())
        );
    }

    #[test]
    fn a_redirect_names_its_server() {
        assert_eq!(
            HandlerOutcome::Redirect("hub".into()).into_wit(),
            WitHandlerResult::Redirect("hub".into())
        );
    }
}
