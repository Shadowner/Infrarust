mod admin;
mod chat;
mod client;
mod connection;
mod handshake;
mod limbo;
mod login;
mod messaging;
mod packet;
mod proxy;

pub use admin::{
    BanIssuedEvent, BanRevokedEvent, BanSource, PluginDisabledEvent, PluginEnabledEvent,
};
pub use chat::{ChatMessageEvent, ChatMessageResult, CommandExecuteEvent, CommandExecuteResult};
pub use client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerResourcePackStatusEvent,
    PlayerSettingsChangedEvent, PreTransferEvent, PreTransferResult, ResourcePackOrigin,
    ResourcePackStatus, TransferOrigin,
};
pub use connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent, ServerPreConnectResult,
};
pub use handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};
pub use limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
pub use login::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult,
    OnlineAuthFailedEvent, PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent,
    PreLoginEvent, PreLoginResult,
};
pub use messaging::{
    MessageEndpoint, MessagePhase, NamedEvent, NamedOutcome, NamedResponse, PluginMessageEvent,
    PluginMessageResult,
};
pub use packet::{PacketFilter, RawPacketEvent, RawPacketResult};
pub use proxy::{
    BackendHealthEvent, BackendState, ConfigReloadEvent, PingResponse, ProxyInitializeEvent,
    ProxyPingEvent, ProxyShutdownEvent, ServerStateChangeEvent,
};

use crate::bindings::events::{Event, EventKind, EventOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPriority {
    First,
    Early,
    Normal,
    Late,
    Last,
    Custom(u8),
}

impl EventPriority {
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::First => 0,
            Self::Early => 64,
            Self::Normal => 128,
            Self::Late => 192,
            Self::Last => 255,
            Self::Custom(v) => v,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultCell<R> {
    current: R,
    dirty: bool,
}

impl<R> ResultCell<R> {
    #[must_use]
    pub const fn new(current: R) -> Self {
        Self {
            current,
            dirty: false,
        }
    }

    #[must_use]
    pub const fn get(&self) -> &R {
        &self.current
    }

    pub fn set(&mut self, value: R) {
        self.current = value;
        self.dirty = true;
    }

    pub fn get_mut(&mut self) -> &mut R {
        self.dirty = true;
        &mut self.current
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    #[must_use]
    pub fn into_changed(self) -> Option<R> {
        self.dirty.then_some(self.current)
    }
}

pub trait GuestEvent: Sized + 'static {
    const KIND: EventKind;

    #[doc(hidden)]
    fn from_event(ev: Event) -> Option<Self>;

    #[doc(hidden)]
    fn into_outcome(self) -> EventOutcome {
        EventOutcome::Unchanged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_cell_is_unchanged_until_touched() {
        let untouched = ResultCell::new(1);
        assert_eq!(*untouched.get(), 1);
        assert!(!untouched.is_dirty());
        assert_eq!(untouched.into_changed(), None);

        let mut set = ResultCell::new(1);
        set.set(1);
        assert_eq!(
            set.into_changed(),
            Some(1),
            "setting the same value still counts"
        );

        let mut edited = ResultCell::new(vec![1]);
        edited.get_mut().push(2);
        assert_eq!(edited.into_changed(), Some(vec![1, 2]));
    }

    #[test]
    fn priorities_match_the_native_levels() {
        assert_eq!(EventPriority::First.value(), 0);
        assert_eq!(EventPriority::Normal.value(), 128);
        assert_eq!(EventPriority::Custom(32).value(), 32);
    }
}
