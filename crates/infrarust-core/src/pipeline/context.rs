use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use bytes::BytesMut;
use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_transport::{AcceptedConnection, ConnectionInfo};

use crate::error::CoreError;

/// Type-erased extension map for storing middleware data.
///
/// Inspired by the rama Extensions pattern: a `HashMap<TypeId, Box<dyn Any>>`
/// allowing middlewares to insert and retrieve typed data without coupling.
#[derive(Debug, Default)]
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Extensions {
    /// Creates an empty extensions map.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Inserts a value of type `T`, returning the previous value if one existed.
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(|prev| prev.downcast().ok().map(|b| *b))
    }

    /// Returns a reference to the value of type `T`, if present.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref())
    }

    /// Returns a mutable reference to the value of type `T`, if present.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut())
    }

    /// Returns `true` if a value of type `T` is stored.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }

    /// Removes and returns the value of type `T`, if present.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast().ok().map(|b| *b))
    }
}

/// Per-connection context threaded through the middleware pipeline.
///
/// Fixed fields are set at accept time. Middlewares enrich the context
/// by inserting typed data into `extensions`.
pub struct ConnectionContext {
    /// Client TCP stream. `Option` so passthrough handler can take ownership.
    stream: Option<TcpStream>,
    /// Peer socket address (may be load balancer address).
    pub peer_addr: SocketAddr,
    /// Effective client IP (after proxy protocol resolution).
    pub client_ip: IpAddr,
    /// Local listener address.
    pub local_addr: SocketAddr,
    /// Timestamp when the connection was accepted.
    pub connected_at: Instant,
    /// Bytes already read from the stream (proxy protocol header, partial handshake).
    pub buffered_data: BytesMut,
    /// Type-erased data accumulated by middlewares.
    pub extensions: Extensions,
}

impl ConnectionContext {
    /// Constructs a context from an accepted transport connection.
    pub fn from_accepted(accepted: AcceptedConnection) -> Self {
        let peer_addr = accepted.connection.peer_addr();
        let client_ip = accepted.connection.client_addr();
        let local_addr = accepted.connection.local_addr();
        let connected_at = accepted.connection.connected_at();
        let (stream, buffered_data, _info) = accepted.connection.into_parts();

        Self {
            stream: Some(stream),
            peer_addr,
            client_ip,
            local_addr,
            connected_at,
            buffered_data,
            extensions: Extensions::new(),
        }
    }

    /// Retrieves a required extension, returning an error if missing.
    ///
    /// Prefer this over `extensions.get::<T>().expect(...)` in production code
    /// to avoid panics on pipeline misconfiguration.
    ///
    /// # Errors
    /// Returns `CoreError::MissingExtension` if the extension is not present.
    pub fn require_extension<T: Send + Sync + 'static>(
        &self,
        name: &'static str,
    ) -> Result<&T, CoreError> {
        self.extensions
            .get::<T>()
            .ok_or(CoreError::MissingExtension(name))
    }

    /// Returns a reference to the client stream.
    ///
    /// # Panics
    /// Panics if the stream has already been taken.
    #[allow(clippy::expect_used)] // Intentional panic: taking a stream twice is a programming error
    pub const fn stream(&self) -> &TcpStream {
        self.stream.as_ref().expect("stream already taken")
    }

    /// Returns a mutable reference to the client stream.
    ///
    /// # Panics
    /// Panics if the stream has already been taken.
    #[allow(clippy::expect_used)] // Intentional panic: taking a stream twice is a programming error
    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        self.stream.as_mut().expect("stream already taken")
    }

    /// Takes ownership of the client stream (for passthrough forwarding).
    ///
    /// # Panics
    /// Panics if the stream has already been taken.
    #[allow(clippy::expect_used)] // Intentional panic: taking a stream twice is a programming error
    pub const fn take_stream(&mut self) -> TcpStream {
        self.stream.take().expect("stream already taken")
    }

    /// Returns `true` if the stream is still present.
    pub const fn has_stream(&self) -> bool {
        self.stream.is_some()
    }

    /// Creates a context for testing with a pre-existing TCP stream.
    ///
    /// This avoids needing an `AcceptedConnection` in tests.
    pub fn new_for_test(
        stream: TcpStream,
        peer_addr: SocketAddr,
        client_ip: IpAddr,
        local_addr: SocketAddr,
    ) -> Self {
        Self {
            stream: Some(stream),
            peer_addr,
            client_ip,
            local_addr,
            connected_at: Instant::now(),
            buffered_data: BytesMut::new(),
            extensions: Extensions::new(),
        }
    }

    pub fn connection_duration(&self) -> Duration {
        self.connected_at.elapsed()
    }

    /// Builds a `ConnectionInfo` for passing to `BackendConnector`.
    pub const fn connection_info(&self) -> ConnectionInfo {
        ConnectionInfo {
            peer_addr: self.peer_addr,
            real_ip: Some(self.client_ip),
            real_port: None,
            local_addr: self.local_addr,
            connected_at: self.connected_at,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn extensions_insert_and_get() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        assert_eq!(ext.get::<u32>(), Some(&42));
    }

    #[test]
    fn extensions_get_missing_returns_none() {
        let ext = Extensions::new();
        assert_eq!(ext.get::<u32>(), None);
    }

    #[test]
    fn extensions_overwrite_returns_previous() {
        let mut ext = Extensions::new();
        ext.insert(1u32);
        let prev = ext.insert(2u32);
        assert_eq!(prev, Some(1));
        assert_eq!(ext.get::<u32>(), Some(&2));
    }

    #[test]
    fn extensions_different_types_coexist() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        ext.insert("hello".to_string());
        assert_eq!(ext.get::<u32>(), Some(&42));
        assert_eq!(ext.get::<String>(), Some(&"hello".to_string()));
    }

    #[test]
    fn extensions_contains() {
        let mut ext = Extensions::new();
        assert!(!ext.contains::<u32>());
        ext.insert(1u32);
        assert!(ext.contains::<u32>());
    }

    #[test]
    fn extensions_remove() {
        let mut ext = Extensions::new();
        ext.insert(42u32);
        let removed = ext.remove::<u32>();
        assert_eq!(removed, Some(42));
        assert!(!ext.contains::<u32>());
    }

    #[test]
    fn extensions_get_mut() {
        let mut ext = Extensions::new();
        ext.insert(vec![1, 2, 3]);
        ext.get_mut::<Vec<i32>>().unwrap().push(4);
        assert_eq!(ext.get::<Vec<i32>>(), Some(&vec![1, 2, 3, 4]));
    }
}
