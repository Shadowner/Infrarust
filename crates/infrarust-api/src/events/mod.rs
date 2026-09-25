//! Concrete event definitions.
//!
//! Events are grouped by category:
//! - [`lifecycle`] — Login, post-login, disconnect
//! - [`connection`] — Server routing and kicks
//! - [`proxy`] — Proxy-level events (ping, init, shutdown, config)
//! - [`chat`] — Chat message interception
//! - [`packet`] — Raw packet events (Tier 3)

pub mod ban;
pub mod chat;
pub mod command;
pub mod connection;
pub mod lifecycle;
pub mod named;
pub mod packet;
pub mod proxy;

pub use ban::{BanIssuedEvent, BanRevokedEvent};
pub use chat::{ChatMessageEvent, ChatMessageResult};
pub use command::{CommandExecuteEvent, CommandExecuteResult};
pub use connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent, ServerPreConnectResult,
};
pub use lifecycle::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult,
    OnlineAuthFailed, PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent, PreLoginEvent,
    PreLoginResult,
};
pub use named::{NamedEvent, NamedEventResponse};
pub use packet::{PacketDirection, RawPacketEvent, RawPacketResult};
pub use proxy::{
    BackendHealthEvent, ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
