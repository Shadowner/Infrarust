//! Low-level networking layer for the Infrarust Minecraft proxy.
//!
//! This crate provides TCP accept loops, client/backend connections,
//! HAProxy proxy protocol support, bidirectional forwarding
//! (userspace copy and kernel splice), and advanced socket configuration.

pub mod backend;
pub mod connection;
pub mod error;
pub mod forward;
pub mod listener;
pub mod proxy_protocol;
pub mod socket;

pub use backend::{BackendConnection, BackendConnector};
pub use connection::{ClientConnection, ConnectionInfo};
pub use error::{TransportError, TransportResult};
pub use forward::{CopyForwarder, ForwardEndReason, ForwardResult, Forwarder, select_forwarder};
pub use listener::{AcceptedConnection, Listener, ListenerConfig};
pub use proxy_protocol::{
    ProxyProtocolInfo, ProxyProtocolVersion, decode_proxy_protocol, encode_proxy_protocol_v2,
};
pub use socket::{
    configure_listener_socket, configure_stream_socket, into_tokio_listener, into_tokio_stream,
};

#[cfg(target_os = "linux")]
pub use forward::SpliceForwarder;
