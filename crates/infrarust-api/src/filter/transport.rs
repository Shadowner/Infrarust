//! Transport-level filter types.
//!
//! Transport filters operate on raw TCP bytes, before Minecraft framing.
//! They see ALL connections (including passthrough) and can reject
//! connections at the TCP level.

use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use bytes::BytesMut;

use crate::event::BoxFuture;
use crate::types::Extensions;

use super::metadata::FilterMetadata;

/// Operates on the raw TCP byte stream, BEFORE Minecraft framing.
///
/// Sees all connections including passthrough. Async methods are
/// acceptable here since this is not on the per-packet hot path.
///
/// Not exposed to WASM plugins (too dangerous — direct byte access).
pub trait TransportFilter: Send + Sync {
    /// Returns the filter's metadata for ordering.
    fn metadata(&self) -> FilterMetadata;

    /// Called when a new TCP connection is accepted.
    ///
    /// Return [`FilterVerdict::Reject`] to immediately close the connection.
    fn on_accept<'a>(&'a self, ctx: &'a mut TransportContext) -> BoxFuture<'a, FilterVerdict>;

    /// Incoming bytes from the client, before any processing.
    fn on_client_data<'a>(
        &'a self,
        ctx: &'a mut TransportContext,
        data: &'a mut BytesMut,
    ) -> BoxFuture<'a, FilterVerdict>;

    /// Outgoing bytes to the client.
    fn on_server_data<'a>(
        &'a self,
        ctx: &'a mut TransportContext,
        data: &'a mut BytesMut,
    ) -> BoxFuture<'a, FilterVerdict>;

    /// Called when the connection is closed.
    fn on_close(&self, _ctx: &TransportContext) {}
}

/// Context for a transport-level connection.
#[derive(Debug)]
pub struct TransportContext {
    /// The remote peer's address.
    pub remote_addr: SocketAddr,
    /// The local address this connection was accepted on.
    pub local_addr: SocketAddr,
    /// The real client IP (if behind a proxy / PROXY protocol).
    pub real_ip: Option<IpAddr>,
    /// When the connection was accepted.
    pub connection_time: Instant,
    /// Total bytes received from client so far.
    pub bytes_received: u64,
    /// Total bytes sent to client so far.
    pub bytes_sent: u64,
    /// Unique connection identifier.
    pub connection_id: u64,
    /// Type-erased storage shared between filters for this connection.
    pub extensions: Extensions,
}

/// The verdict returned by a transport filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FilterVerdict {
    /// Continue processing (no changes).
    Continue,
    /// Data was modified, continue processing.
    Modified,
    /// Reject the connection / data.
    Reject,
}
