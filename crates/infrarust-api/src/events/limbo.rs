use std::sync::Arc;

use crate::event::Event;
use crate::limbo::context::LimboEntryContext;
use crate::player::Player;
use crate::types::{Component, PlayerId, ServerId};

#[non_exhaustive]
pub struct LimboEnterEvent {
    pub player: Arc<dyn Player>,
    pub handlers: Vec<String>,
    pub context: LimboEntryContext,
}

impl LimboEnterEvent {
    pub fn new(player: Arc<dyn Player>, handlers: Vec<String>, context: LimboEntryContext) -> Self {
        Self {
            player,
            handlers,
            context,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for LimboEnterEvent {}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LimboExitReason {
    Released,
    Redirected,
    SentToLimbo { handlers: Vec<String> },
    Kicked { reason: Component },
    Disconnected,
    TimedOut,
    Shutdown,
}

impl LimboExitReason {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Redirected => "redirected",
            Self::SentToLimbo { .. } => "sent_to_limbo",
            Self::Kicked { .. } => "kicked",
            Self::Disconnected => "disconnected",
            Self::TimedOut => "timed_out",
            Self::Shutdown => "shutdown",
        }
    }
}

#[non_exhaustive]
pub struct LimboExitEvent {
    pub player: Arc<dyn Player>,
    pub reason: LimboExitReason,
    pub next_server: Option<ServerId>,
}

impl LimboExitEvent {
    pub fn new(
        player: Arc<dyn Player>,
        reason: LimboExitReason,
        next_server: Option<ServerId>,
    ) -> Self {
        Self {
            player,
            reason,
            next_server,
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }
}

impl Event for LimboExitEvent {}
