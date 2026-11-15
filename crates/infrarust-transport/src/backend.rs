//! Backend connection management.
//!
//! `BackendConnector` handles connecting to backend Minecraft servers
//! with failover, timeout, and optional proxy protocol v2.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::Instant;

use infrarust_config::{KeepaliveConfig, ServerAddress};

use crate::connection::ConnectionInfo;
use crate::error::TransportError;
use crate::proxy_protocol::encode_proxy_protocol_v2;
use crate::socket::configure_tokio_stream;

const DNS_CACHE_TTL: Duration = Duration::from_secs(15);
const DNS_CACHE_CAPACITY: usize = 256;

static DNS_CACHE: LazyLock<DnsCache> = LazyLock::new(DnsCache::default);

struct Resolution {
    addrs: Arc<[SocketAddr]>,
    resolved_at: Instant,
}

impl Resolution {
    fn is_fresh(&self) -> bool {
        self.resolved_at.elapsed() < DNS_CACHE_TTL
    }
}

#[derive(Default)]
struct DnsCache {
    entries: Mutex<HashMap<ServerAddress, Resolution>>,
}

impl DnsCache {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<ServerAddress, Resolution>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self, address: &ServerAddress) -> Option<Arc<[SocketAddr]>> {
        self.entries()
            .get(address)
            .filter(|resolution| resolution.is_fresh())
            .map(|resolution| Arc::clone(&resolution.addrs))
    }

    fn insert(&self, address: &ServerAddress, addrs: Arc<[SocketAddr]>) {
        let mut entries = self.entries();
        if entries.len() >= DNS_CACHE_CAPACITY && !entries.contains_key(address) {
            entries.retain(|_, resolution| resolution.is_fresh());
            if entries.len() >= DNS_CACHE_CAPACITY {
                entries.clear();
            }
        }
        entries.insert(
            address.clone(),
            Resolution {
                addrs,
                resolved_at: Instant::now(),
            },
        );
    }

    fn evict(&self, address: &ServerAddress) {
        self.entries().remove(address);
    }
}

fn ip_literal(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

async fn connect_any(addrs: &[SocketAddr]) -> io::Result<TcpStream> {
    let mut last_error = None;
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve to any address",
        )
    }))
}

async fn lookup(address: &ServerAddress) -> io::Result<Arc<[SocketAddr]>> {
    Ok(
        tokio::net::lookup_host((address.host.as_str(), address.port))
            .await?
            .collect(),
    )
}

async fn connect_resolved<F>(
    cache: &DnsCache,
    address: &ServerAddress,
    resolve: impl FnOnce() -> F,
) -> io::Result<TcpStream>
where
    F: Future<Output = io::Result<Arc<[SocketAddr]>>>,
{
    if let Some(addrs) = cache.get(address)
        && let Ok(stream) = connect_any(&addrs).await
    {
        return Ok(stream);
    }

    let addrs = resolve().await?;
    let stream = connect_any(&addrs).await?;
    cache.insert(address, addrs);
    Ok(stream)
}

struct EvictUnlessConnected<'a> {
    cache: &'a DnsCache,
    address: &'a ServerAddress,
    connected: bool,
}

impl Drop for EvictUnlessConnected<'_> {
    fn drop(&mut self) {
        if !self.connected {
            self.cache.evict(self.address);
        }
    }
}

async fn connect_stream(address: &ServerAddress) -> io::Result<TcpStream> {
    connect_stream_with(&DNS_CACHE, address, || lookup(address)).await
}

async fn connect_stream_with<F>(
    cache: &DnsCache,
    address: &ServerAddress,
    resolve: impl FnOnce() -> F,
) -> io::Result<TcpStream>
where
    F: Future<Output = io::Result<Arc<[SocketAddr]>>>,
{
    if let Some(ip) = ip_literal(&address.host) {
        return TcpStream::connect(SocketAddr::new(ip, address.port)).await;
    }
    let mut guard = EvictUnlessConnected {
        cache,
        address,
        connected: false,
    };
    let stream = connect_resolved(cache, address, resolve).await?;
    guard.connected = true;
    Ok(stream)
}

pub struct ConnectAttempt<'a> {
    pub server_id: &'a str,
    pub address: &'a ServerAddress,
    pub elapsed: Duration,
    pub error: Option<&'a TransportError>,
}

impl ConnectAttempt<'_> {
    pub const fn succeeded(&self) -> bool {
        self.error.is_none()
    }
}

pub trait ConnectAttemptObserver: Send + Sync {
    fn on_attempt(&self, attempt: &ConnectAttempt<'_>);
}

/// Connects to backend servers with failover and timeout.
#[derive(Clone)]
pub struct BackendConnector {
    /// Default connection timeout.
    pub default_timeout: Duration,
    /// TCP keepalive configuration for backend connections.
    pub keepalive: KeepaliveConfig,
    /// Addresses tried before giving up. `0` means no limit.
    max_attempts: usize,
    /// Notified of every connection attempt outcome (per address).
    observer: Option<Arc<dyn ConnectAttemptObserver>>,
}

impl std::fmt::Debug for BackendConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendConnector")
            .field("default_timeout", &self.default_timeout)
            .field("keepalive", &self.keepalive)
            .field("max_attempts", &self.max_attempts)
            .field("has_observer", &self.observer.is_some())
            .finish()
    }
}

impl BackendConnector {
    pub const fn new(default_timeout: Duration, keepalive: KeepaliveConfig) -> Self {
        Self {
            default_timeout,
            keepalive,
            max_attempts: 0,
            observer: None,
        }
    }

    #[must_use]
    pub const fn with_max_attempts(mut self, max_attempts: usize) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn ConnectAttemptObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Connects to one of the given backend addresses.
    ///
    /// Tries each address in order. On the first successful connection,
    /// configures the socket (`TCP_NODELAY`, keepalive) and optionally
    /// sends a proxy protocol v2 header.
    ///
    /// Returns `AllBackendsFailed` if no address could be reached.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::AllBackendsFailed`] if none of the provided
    /// addresses could be reached, or a connection-specific error from the
    /// last failed attempt.
    pub async fn connect(
        &self,
        server_id: &str,
        addresses: &[ServerAddress],
        timeout_override: Option<Duration>,
        send_proxy_protocol: bool,
        client_info: &ConnectionInfo,
    ) -> Result<BackendConnection, TransportError> {
        let timeout = timeout_override.unwrap_or(self.default_timeout);
        let mut last_error = None;

        let attempts = if self.max_attempts == 0 {
            addresses.len()
        } else {
            self.max_attempts.min(addresses.len())
        };

        for address in &addresses[..attempts] {
            let started = Instant::now();
            let outcome = self
                .try_connect(address, timeout, send_proxy_protocol, client_info)
                .await;
            let elapsed = started.elapsed();

            match outcome {
                Ok(conn) => {
                    if let Some(ref observer) = self.observer {
                        observer.on_attempt(&ConnectAttempt {
                            server_id,
                            address,
                            elapsed,
                            error: None,
                        });
                    }
                    return Ok(conn);
                }
                Err(e) => {
                    tracing::warn!(
                        server_id = server_id,
                        address = %address,
                        error = %e,
                        "backend connection attempt failed"
                    );
                    if let Some(ref observer) = self.observer {
                        observer.on_attempt(&ConnectAttempt {
                            server_id,
                            address,
                            elapsed,
                            error: Some(&e),
                        });
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(
            last_error.unwrap_or_else(|| TransportError::AllBackendsFailed {
                server_id: server_id.to_string(),
            }),
        )
    }

    pub async fn connect_one(
        &self,
        address: &ServerAddress,
        timeout: Duration,
        send_proxy_protocol: bool,
        client_info: &ConnectionInfo,
    ) -> Result<BackendConnection, TransportError> {
        self.try_connect(address, timeout, send_proxy_protocol, client_info)
            .await
    }

    async fn try_connect(
        &self,
        address: &ServerAddress,
        timeout: Duration,
        send_proxy_protocol: bool,
        client_info: &ConnectionInfo,
    ) -> Result<BackendConnection, TransportError> {
        let mut stream = tokio::time::timeout(timeout, connect_stream(address))
            .await
            .map_err(|_| TransportError::ConnectTimeout {
                addr: address.to_string(),
                timeout,
            })?
            .map_err(|source| TransportError::BackendConnect {
                addr: address.to_string(),
                source,
            })?;

        let remote_addr = stream.peer_addr().map_err(TransportError::SocketConfig)?;
        configure_tokio_stream(&stream, &self.keepalive)?;

        if send_proxy_protocol {
            encode_proxy_protocol_v2(&mut stream, client_info).await?;
        }

        Ok(BackendConnection {
            stream,
            remote_addr,
            server_address: address.clone(),
            connected_at: Instant::now(),
        })
    }
}

/// An established connection to a backend server.
#[derive(Debug)]
pub struct BackendConnection {
    stream: TcpStream,
    remote_addr: SocketAddr,
    server_address: ServerAddress,
    connected_at: Instant,
}

impl BackendConnection {
    /// Consumes the connection and returns the TCP stream.
    pub fn into_stream(self) -> TcpStream {
        self.stream
    }

    /// Returns a reference to the TCP stream.
    pub const fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Returns a mutable reference to the TCP stream.
    pub const fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }

    /// Returns the remote backend address.
    pub const fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    pub const fn server_address(&self) -> &ServerAddress {
        &self.server_address
    }

    /// Returns the time when the connection was established.
    pub const fn connected_at(&self) -> Instant {
        self.connected_at
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::net::TcpListener;

    use super::*;

    const HOST: &str = "backend.test";

    async fn listening() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    async fn refusing() -> SocketAddr {
        listening().await.1
    }

    fn hostname(port: u16) -> ServerAddress {
        ServerAddress {
            host: HOST.to_string(),
            port,
        }
    }

    struct Resolver {
        addrs: Arc<[SocketAddr]>,
        calls: AtomicUsize,
    }

    impl Resolver {
        fn to(addrs: &[SocketAddr]) -> Self {
            Self {
                addrs: addrs.into(),
                calls: AtomicUsize::new(0),
            }
        }

        async fn resolve(&self) -> io::Result<Arc<[SocketAddr]>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::clone(&self.addrs))
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    async fn connect(
        cache: &DnsCache,
        address: &ServerAddress,
        resolver: &Resolver,
    ) -> io::Result<SocketAddr> {
        connect_stream_with(cache, address, || resolver.resolve())
            .await?
            .peer_addr()
    }

    #[tokio::test]
    async fn an_ip_literal_bypasses_the_resolver_and_the_cache() {
        let (_listener, live) = listening().await;
        let cache = DnsCache::default();
        let resolver = Resolver::to(&[]);

        let address = ServerAddress {
            host: live.ip().to_string(),
            port: live.port(),
        };
        assert_eq!(connect(&cache, &address, &resolver).await.unwrap(), live);

        let dead = refusing().await;
        let address = ServerAddress {
            host: dead.ip().to_string(),
            port: dead.port(),
        };
        connect(&cache, &address, &resolver).await.unwrap_err();

        assert_eq!(resolver.calls(), 0);
        assert!(cache.entries().is_empty());
    }

    #[test]
    fn bracketed_ipv6_hosts_are_ip_literals() {
        assert_eq!(
            ip_literal("[::1]"),
            Some(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]))
        );
        assert_eq!(
            ip_literal("::1"),
            Some(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]))
        );
        assert_eq!(ip_literal("10.0.0.7"), Some(IpAddr::from([10, 0, 0, 7])));
        assert_eq!(ip_literal(HOST), None);
        assert_eq!(ip_literal("[backend.test]"), None);
    }

    #[tokio::test]
    async fn a_hostname_is_resolved_once_within_the_ttl() {
        let (_listener, live) = listening().await;
        let cache = DnsCache::default();
        let resolver = Resolver::to(&[live]);
        let address = hostname(live.port());

        for _ in 0..3 {
            assert_eq!(connect(&cache, &address, &resolver).await.unwrap(), live);
        }

        assert_eq!(resolver.calls(), 1);
        assert_eq!(cache.get(&address).as_deref(), Some(&[live][..]));
    }

    #[tokio::test(start_paused = true)]
    async fn a_hostname_is_resolved_again_once_the_ttl_has_elapsed() {
        let (_listener, live) = listening().await;
        let cache = DnsCache::default();
        let resolver = Resolver::to(&[live]);
        let address = hostname(live.port());

        connect(&cache, &address, &resolver).await.unwrap();
        tokio::time::advance(DNS_CACHE_TTL - Duration::from_millis(1)).await;
        connect(&cache, &address, &resolver).await.unwrap();
        assert_eq!(resolver.calls(), 1);

        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(cache.get(&address).is_none());
        connect(&cache, &address, &resolver).await.unwrap();
        assert_eq!(resolver.calls(), 2);
        assert!(cache.get(&address).is_some());
    }

    #[tokio::test]
    async fn a_failed_connect_evicts_the_hostname() {
        let dead = refusing().await;
        let cache = DnsCache::default();
        let resolver = Resolver::to(&[dead]);
        let address = hostname(dead.port());
        let other = hostname(dead.port().wrapping_add(1));
        cache.insert(&address, [dead].into());
        cache.insert(&other, [dead].into());

        connect(&cache, &address, &resolver).await.unwrap_err();

        assert_eq!(resolver.calls(), 1);
        assert!(cache.get(&address).is_none());
        assert!(cache.get(&other).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_connect_evicts_the_hostname() {
        let dead = refusing().await;
        let cache = DnsCache::default();
        let address = hostname(dead.port());
        cache.insert(&address, [dead].into());

        let attempt = connect_stream_with(&cache, &address, std::future::pending);
        tokio::time::timeout(Duration::from_secs(1), attempt)
            .await
            .unwrap_err();

        assert!(cache.get(&address).is_none());
    }

    #[tokio::test]
    async fn failing_cached_addresses_fall_back_to_a_fresh_resolve() {
        let dead = refusing().await;
        let (_listener, live) = listening().await;
        let cache = DnsCache::default();
        let resolver = Resolver::to(&[dead, live]);
        let address = hostname(live.port());
        cache.insert(&address, [dead].into());

        assert_eq!(connect(&cache, &address, &resolver).await.unwrap(), live);

        assert_eq!(resolver.calls(), 1);
        assert_eq!(cache.get(&address).as_deref(), Some(&[dead, live][..]));

        assert_eq!(connect(&cache, &address, &resolver).await.unwrap(), live);
        assert_eq!(resolver.calls(), 1);
    }
}
