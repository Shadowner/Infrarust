//! Concrete event definitions.
//!
//! Events are grouped by category:
//! - [`lifecycle`] — Login, post-login, disconnect
//! - [`connection`] — Server routing and kicks
//! - [`proxy`] — Proxy-level events (ping, init, shutdown, config)
//! - [`chat`] — Chat message interception
//! - [`packet`] — Raw packet events (Tier 3)

pub mod chat;
pub mod connection;
pub mod lifecycle;
pub mod packet;
pub mod proxy;

pub use chat::{ChatMessageEvent, ChatMessageResult};
pub use connection::{
    KickedFromServerEvent, KickedFromServerResult, ServerConnectedEvent, ServerPreConnectEvent,
    ServerPreConnectResult, ServerSwitchEvent,
};
pub use lifecycle::{DisconnectEvent, PostLoginEvent, PreLoginEvent, PreLoginResult};
pub use packet::{PacketDirection, RawPacketEvent, RawPacketResult};
pub use proxy::{
    ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent, ProxyShutdownEvent,
    ServerStateChangeEvent,
};
