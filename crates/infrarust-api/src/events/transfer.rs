use std::sync::Arc;

use crate::event::{Event, ResultedEvent};
use crate::player::Player;
use crate::types::{Component, PlayerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransferOrigin {
    Plugin,
    Backend,
}

impl TransferOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plugin => "plugin",
            Self::Backend => "backend",
        }
    }
}

#[non_exhaustive]
pub struct PreTransferEvent {
    pub player: Arc<dyn Player>,
    pub host: String,
    pub port: u16,
    pub origin: TransferOrigin,
    result: PreTransferResult,
}

impl PreTransferEvent {
    pub fn new(player: Arc<dyn Player>, host: String, port: u16, origin: TransferOrigin) -> Self {
        Self {
            player,
            host,
            port,
            origin,
            result: PreTransferResult::default(),
        }
    }

    pub fn player_id(&self) -> PlayerId {
        self.player.id()
    }

    pub fn deny(&mut self, reason: Component) {
        self.result = PreTransferResult::Denied { reason };
    }

    pub fn redirect(&mut self, host: impl Into<String>, port: u16) {
        self.result = PreTransferResult::Redirect {
            host: host.into(),
            port,
        };
    }

    pub fn destination(&self) -> Option<(&str, u16)> {
        match &self.result {
            PreTransferResult::Denied { .. } => None,
            PreTransferResult::Redirect { host, port } => Some((host, *port)),
            _ => Some((&self.host, self.port)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub enum PreTransferResult {
    #[default]
    Allowed,
    Denied {
        reason: Component,
    },
    Redirect {
        host: String,
        port: u16,
    },
}

impl Event for PreTransferEvent {}

impl ResultedEvent for PreTransferEvent {
    type Result = PreTransferResult;

    fn result(&self) -> &Self::Result {
        &self.result
    }

    fn set_result(&mut self, result: Self::Result) {
        self.result = result;
    }
}
