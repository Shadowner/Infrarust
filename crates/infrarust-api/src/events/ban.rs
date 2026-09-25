use crate::event::Event;
use crate::services::ban_service::{BanEntry, BanSource};

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BanIssuedEvent {
    pub entry: BanEntry,
    pub source: BanSource,
    pub silent: bool,
}

impl BanIssuedEvent {
    pub const fn new(entry: BanEntry, source: BanSource, silent: bool) -> Self {
        Self {
            entry,
            source,
            silent,
        }
    }
}

impl Event for BanIssuedEvent {}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BanRevokedEvent {
    pub entry: BanEntry,
    pub source: BanSource,
    pub silent: bool,
}

impl BanRevokedEvent {
    pub const fn new(entry: BanEntry, source: BanSource, silent: bool) -> Self {
        Self {
            entry,
            source,
            silent,
        }
    }
}

impl Event for BanRevokedEvent {}
