//! Low-level networking layer for the Infrarust Minecraft proxy.
//!
//! This crate provides TCP accept loops, client/backend connections,
//! `HAProxy` proxy protocol support, bidirectional forwarding
//! (userspace copy and kernel splice), and advanced socket configuration.

pub mod backend;
pub mod connection;
pub mod error;
pub mod forward;
pub mod ip_limit;
pub mod listener;
pub mod proxy_protocol;
pub mod socket;

pub use backend::{BackendConnection, BackendConnector, ConnectAttempt, ConnectAttemptObserver};
pub use connection::{ClientConnection, ConnectionInfo};
pub use error::{TransportError, TransportResult};
pub use forward::{ForwardEndReason, ForwardResult, Forwarder};
pub use ip_limit::{IpGuard, IpKey, IpLimiter};
pub use listener::{
    AcceptedConnection, ConnectionSlot, Listener, ListenerConfig, PendingConnection,
};
pub use proxy_protocol::{ProxyProtocolInfo, ProxyProtocolVersion};
pub use socket::activated_listener;
