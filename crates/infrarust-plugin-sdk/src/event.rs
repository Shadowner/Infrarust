//! Typed events over the raw `handle-event` dispatch.
//!
//! Each event kind maps to a Rust type implementing [`GuestEvent`]. Observe-only
//! kinds are the generated records; the six modifiable kinds are wrappers that
//! carry an optional result a handler sets via `deny`/`redirect_to`/etc. An
//! unset result yields [`EventOutcome::None`] so the host keeps native behavior.
//!
//! Carried by the WIT contract but not yet exposed here (planned):
//! the `raw-packet` kind has no typed [`GuestEvent`] wrapper, and
//! [`PermissionsSetupEvent`] is observe-only — a custom permission checker
//! cannot be registered from WASM yet (the generated glue answers
//! `Player`/`false` for the permission dispatch).

use crate::bindings::event_bus::EventKind;
use crate::bindings::guest::{self, Event, EventOutcome};
use crate::bindings::types::GameProfile;

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

/// An event a plugin can handle.
pub trait GuestEvent: Sized + 'static {
    /// The wire kind, passed to `event-bus.subscribe`.
    const KIND: EventKind;
    #[doc(hidden)]
    fn from_event(ev: Event) -> Option<Self>;
    #[doc(hidden)]
    fn into_outcome(self) -> EventOutcome;
}

macro_rules! observe_records {
    ($($kind:ident => $ty:ident),* $(,)?) => {
        $(
            pub use crate::bindings::guest::$ty;
            impl GuestEvent for $ty {
                const KIND: EventKind = EventKind::$kind;
                fn from_event(ev: Event) -> Option<Self> {
                    if let Event::$kind(e) = ev { Some(e) } else { None }
                }
                fn into_outcome(self) -> EventOutcome { EventOutcome::None }
            }
        )*
    };
}

observe_records! {
    PostLogin => PostLoginEvent,
    Disconnect => DisconnectEvent,
    OnlineAuthFailed => OnlineAuthFailedEvent,
    PermissionsSetup => PermissionsSetupEvent,
    ServerConnected => ServerConnectedEvent,
    ServerSwitch => ServerSwitchEvent,
    ServerStateChange => ServerStateChangeEvent,
}

macro_rules! observe_units {
    ($($kind:ident => $ty:ident),* $(,)?) => {
        $(
            #[doc = concat!("The data-less `", stringify!($kind), "` event.")]
            pub struct $ty;
            impl GuestEvent for $ty {
                const KIND: EventKind = EventKind::$kind;
                fn from_event(ev: Event) -> Option<Self> {
                    if matches!(ev, Event::$kind) { Some($ty) } else { None }
                }
                fn into_outcome(self) -> EventOutcome { EventOutcome::None }
            }
        )*
    };
}

observe_units! {
    ProxyInitialize => ProxyInitializeEvent,
    ProxyShutdown => ProxyShutdownEvent,
    ConfigReload => ConfigReloadEvent,
}

/// A player tried to log in; a handler may force the auth mode or deny entry.
pub struct PreLoginEvent {
    pub profile: GameProfile,
    pub remote_addr: String,
    pub protocol_version: i32,
    pub server_domain: String,
    result: Option<guest::PreLoginResult>,
}

impl PreLoginEvent {
    pub fn deny(&mut self, reason: impl Into<String>) {
        self.result = Some(guest::PreLoginResult::Denied(reason.into()));
    }
    pub fn force_offline(&mut self) {
        self.result = Some(guest::PreLoginResult::ForceOffline);
    }
    pub fn force_online(&mut self) {
        self.result = Some(guest::PreLoginResult::ForceOnline);
    }
    pub fn allow(&mut self) {
        self.result = None;
    }
}

impl GuestEvent for PreLoginEvent {
    const KIND: EventKind = EventKind::PreLogin;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::PreLogin(e) = ev else { return None };
        Some(Self {
            profile: e.profile,
            remote_addr: e.remote_addr,
            protocol_version: e.protocol_version,
            server_domain: e.server_domain,
            result: None,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        self.result
            .map_or(EventOutcome::None, EventOutcome::PreLogin)
    }
}

/// A player is about to connect to its target server; a handler may redirect or deny.
pub struct ServerPreConnectEvent {
    pub player_id: u64,
    pub profile: GameProfile,
    pub original_server: String,
    result: Option<guest::ServerPreConnectResult>,
}

impl ServerPreConnectEvent {
    pub fn redirect_to(&mut self, server: impl Into<String>) {
        self.result = Some(guest::ServerPreConnectResult::ConnectTo(server.into()));
    }
    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.result = Some(guest::ServerPreConnectResult::SendToLimbo(handlers));
    }
    pub fn deny(&mut self, reason: impl Into<String>) {
        self.result = Some(guest::ServerPreConnectResult::Denied(reason.into()));
    }
    pub fn allow(&mut self) {
        self.result = None;
    }
}

impl GuestEvent for ServerPreConnectEvent {
    const KIND: EventKind = EventKind::ServerPreConnect;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::ServerPreConnect(e) = ev else {
            return None;
        };
        Some(Self {
            player_id: e.player_id,
            profile: e.profile,
            original_server: e.original_server,
            result: None,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        self.result
            .map_or(EventOutcome::None, EventOutcome::ServerPreConnect)
    }
}

/// A player was kicked from a server; a handler may redirect, hold, or notify.
pub struct KickedFromServerEvent {
    pub player_id: u64,
    pub server: String,
    pub reason: String,
    result: Option<guest::KickedFromServerResult>,
}

impl KickedFromServerEvent {
    pub fn disconnect_player(&mut self, reason: impl Into<String>) {
        self.result = Some(guest::KickedFromServerResult::DisconnectPlayer(
            reason.into(),
        ));
    }
    pub fn redirect_to(&mut self, server: impl Into<String>) {
        self.result = Some(guest::KickedFromServerResult::RedirectTo(server.into()));
    }
    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.result = Some(guest::KickedFromServerResult::SendToLimbo(handlers));
    }
    pub fn notify(&mut self, message: impl Into<String>) {
        self.result = Some(guest::KickedFromServerResult::Notify(message.into()));
    }
}

impl GuestEvent for KickedFromServerEvent {
    const KIND: EventKind = EventKind::KickedFromServer;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::KickedFromServer(e) = ev else {
            return None;
        };
        Some(Self {
            player_id: e.player_id,
            server: e.server,
            reason: e.reason,
            result: None,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        self.result
            .map_or(EventOutcome::None, EventOutcome::KickedFromServer)
    }
}

/// A player is choosing its initial server; a handler may redirect or hold in limbo.
pub struct PlayerChooseInitialServerEvent {
    pub player_id: u64,
    pub profile: GameProfile,
    pub initial_server: String,
    result: Option<guest::PlayerChooseInitialServerResult>,
}

impl PlayerChooseInitialServerEvent {
    pub fn redirect(&mut self, server: impl Into<String>) {
        self.result = Some(guest::PlayerChooseInitialServerResult::Redirect(
            server.into(),
        ));
    }
    pub fn send_to_limbo(&mut self, handlers: Vec<String>) {
        self.result = Some(guest::PlayerChooseInitialServerResult::SendToLimbo(
            handlers,
        ));
    }
    pub fn allow(&mut self) {
        self.result = None;
    }
}

impl GuestEvent for PlayerChooseInitialServerEvent {
    const KIND: EventKind = EventKind::PlayerChooseInitialServer;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::PlayerChooseInitialServer(e) = ev else {
            return None;
        };
        Some(Self {
            player_id: e.player_id,
            profile: e.profile,
            initial_server: e.initial_server,
            result: None,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        self.result
            .map_or(EventOutcome::None, EventOutcome::PlayerChooseInitialServer)
    }
}

/// A status ping; a handler mutates [`response`](Self::response) in place.
pub struct ProxyPingEvent {
    pub remote_addr: String,
    pub response: guest::PingResponse,
}

impl GuestEvent for ProxyPingEvent {
    const KIND: EventKind = EventKind::ProxyPing;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::ProxyPing(e) = ev else { return None };
        Some(Self {
            remote_addr: e.remote_addr,
            response: e.response,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        EventOutcome::ProxyPing(self.response)
    }
}

/// A chat message; a handler may deny or rewrite it.
pub struct ChatMessageEvent {
    pub player_id: u64,
    pub message: String,
    result: Option<guest::ChatMessageResult>,
}

impl ChatMessageEvent {
    pub fn deny(&mut self, reason: impl Into<String>) {
        self.result = Some(guest::ChatMessageResult::Deny(reason.into()));
    }
    pub fn modify(&mut self, message: impl Into<String>) {
        self.result = Some(guest::ChatMessageResult::Modify(message.into()));
    }
    pub fn allow(&mut self) {
        self.result = None;
    }
}

impl GuestEvent for ChatMessageEvent {
    const KIND: EventKind = EventKind::ChatMessage;
    fn from_event(ev: Event) -> Option<Self> {
        let Event::ChatMessage(e) = ev else {
            return None;
        };
        Some(Self {
            player_id: e.player_id,
            message: e.message,
            result: None,
        })
    }
    fn into_outcome(self) -> EventOutcome {
        self.result
            .map_or(EventOutcome::None, EventOutcome::ChatMessage)
    }
}
