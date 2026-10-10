use std::sync::Arc;

use crate::player::Player;
use crate::types::Component;

pub use infrarust_plugin_common::enums::TransferOrigin;

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

crate::events::player_event!(PreTransferEvent);

crate::event::resulted_event!(PreTransferEvent, PreTransferResult);
