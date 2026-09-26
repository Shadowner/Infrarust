use super::GuestEvent;
use crate::bindings::ban_service as wb;
use crate::bindings::events::{Event, EventKind};
use crate::services::BanEntry;
use crate::types::{uuid_from_wit, uuid_to_wit};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BanSource {
    Console,
    Player { uuid: uuid::Uuid, name: String },
    Plugin(String),
    WebApi(Option<String>),
    System,
}

impl BanSource {
    pub(crate) fn to_wit(&self) -> wb::BanSource {
        match self {
            Self::Console => wb::BanSource::Console,
            Self::Player { uuid, name } => wb::BanSource::Player(wb::BanActor {
                uuid: uuid_to_wit(*uuid),
                name: name.clone(),
            }),
            Self::Plugin(id) => wb::BanSource::Plugin(id.clone()),
            Self::WebApi(actor) => wb::BanSource::WebApi(actor.clone()),
            Self::System => wb::BanSource::System,
        }
    }

    pub(crate) fn from_wit(source: wb::BanSource) -> Self {
        match source {
            wb::BanSource::Console => Self::Console,
            wb::BanSource::Player(actor) => Self::Player {
                uuid: uuid_from_wit(actor.uuid),
                name: actor.name,
            },
            wb::BanSource::Plugin(id) => Self::Plugin(id),
            wb::BanSource::WebApi(actor) => Self::WebApi(actor),
            wb::BanSource::System => Self::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanIssuedEvent {
    pub entry: BanEntry,
    pub source: BanSource,
    pub silent: bool,
}

impl GuestEvent for BanIssuedEvent {
    const KIND: EventKind = EventKind::BanIssued;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::BanIssued(e) = ev else {
            return None;
        };
        Some(Self {
            entry: BanEntry::from_wit(e.entry),
            source: BanSource::from_wit(e.source),
            silent: e.silent,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BanRevokedEvent {
    pub entry: BanEntry,
    pub source: BanSource,
    pub silent: bool,
}

impl GuestEvent for BanRevokedEvent {
    const KIND: EventKind = EventKind::BanRevoked;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::BanRevoked(e) = ev else {
            return None;
        };
        Some(Self {
            entry: BanEntry::from_wit(e.entry),
            source: BanSource::from_wit(e.source),
            silent: e.silent,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginEnabledEvent {
    pub plugin_id: String,
    pub version: String,
}

impl GuestEvent for PluginEnabledEvent {
    const KIND: EventKind = EventKind::PluginEnabled;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::PluginEnabled(e) = ev else {
            return None;
        };
        Some(Self {
            plugin_id: e.plugin_id,
            version: e.version,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginDisabledEvent {
    pub plugin_id: String,
}

impl GuestEvent for PluginDisabledEvent {
    const KIND: EventKind = EventKind::PluginDisabled;

    fn from_event(ev: Event) -> Option<Self> {
        let Event::PluginDisabled(e) = ev else {
            return None;
        };
        Some(Self {
            plugin_id: e.plugin_id,
        })
    }
}
