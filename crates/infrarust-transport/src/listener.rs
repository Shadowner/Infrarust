use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use ipnet::IpNet;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use infrarust_config::KeepaliveConfig;

use crate::connection::{ClientConnection, canonicalize_ip};
use crate::error::TransportError;
use crate::ip_limit::{IpGuard, IpLimiter};
use crate::proxy_protocol::decode_proxy_protocol;
use crate::socket::{
    adopt_listener_socket, configure_listener_socket, configure_tokio_stream, into_tokio_listener,
};

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub bind: SocketAddr,
    pub max_connections: u32,
    pub max_connections_per_ip: u32,
    pub keepalive: KeepaliveConfig,
    pub so_reuseport: bool,
    pub receive_proxy_protocol: bool,
    pub trusted_proxies: Vec<IpNet>,
}

#[derive(Debug)]
pub struct Listener {
    inner: TcpListener,
    local_addr: SocketAddr,
    semaphore: Option<Arc<Semaphore>>,
    ip_limiter: Option<Arc<IpLimiter>>,
    config: ListenerConfig,
    shutdown: CancellationToken,
}

#[derive(Debug, Clone, Copy)]
enum ProxyHeader {
    NotExpected,
    Expected,
    UntrustedPeer,
}

impl ProxyHeader {
    fn for_peer(config: &ListenerConfig, peer: IpAddr) -> Self {
        if !config.receive_proxy_protocol {
            return Self::NotExpected;
        }
        let peer = canonicalize_ip(peer);
        if config.trusted_proxies.iter().any(|net| net.contains(&peer)) {
            Self::Expected
        } else {
            Self::UntrustedPeer
        }
    }
}

#[derive(Debug)]
pub struct PendingConnection {
    connection: ClientConnection,
    proxy_header: ProxyHeader,
    ip_limiter: Option<Arc<IpLimiter>>,
    permit: Option<OwnedSemaphorePermit>,
}

impl PendingConnection {
    pub const fn peer_addr(&self) -> SocketAddr {
        self.connection.peer_addr()
    }

    pub async fn resolve(self) -> Result<AcceptedConnection, TransportError> {
        let Self {
            mut connection,
            proxy_header,
            ip_limiter,
            permit,
        } = self;
        match proxy_header {
            ProxyHeader::NotExpected => {}
            ProxyHeader::UntrustedPeer => {
                return Err(TransportError::UntrustedProxyPeer {
                    peer: canonicalize_ip(connection.peer_addr().ip()),
                });
            }
            ProxyHeader::Expected => {
                let (info, leftover) = decode_proxy_protocol(connection.stream_mut()).await?;
                if let Some(info) = info {
                    connection = connection.with_proxy_protocol(&info);
                }
                connection.inject_buffered_data(&leftover);
            }
        }
        let ip_guard = match ip_limiter {
            Some(limiter) => {
                let ip = connection.client_addr();
                Some(
                    limiter
                        .try_acquire(ip)
                        .ok_or(TransportError::IpLimitReached { ip })?,
                )
            }
            None => None,
        };
        Ok(AcceptedConnection {
            connection,
            slot: ConnectionSlot {
                _permit: permit,
                _ip_guard: ip_guard,
            },
        })
    }
}

#[derive(Debug, Default)]
pub struct ConnectionSlot {
    _permit: Option<OwnedSemaphorePermit>,
    _ip_guard: Option<IpGuard>,
}

#[derive(Debug)]
pub struct AcceptedConnection {
    connection: ClientConnection,
    slot: ConnectionSlot,
}

impl AcceptedConnection {
    pub fn into_parts(self) -> (ClientConnection, ConnectionSlot) {
        (self.connection, self.slot)
    }
}

impl Listener {
    pub fn bind(
        config: ListenerConfig,
        shutdown: CancellationToken,
    ) -> Result<Self, TransportError> {
        let socket = configure_listener_socket(config.bind, config.so_reuseport)?;
        Self::listen(socket, config, shutdown)
    }

    pub fn adopt(
        inherited: std::net::TcpListener,
        config: ListenerConfig,
        shutdown: CancellationToken,
    ) -> Result<Self, TransportError> {
        let socket = adopt_listener_socket(inherited)?;
        tracing::info!(configured_bind = %config.bind, "adopting socket passed by socket activation");
        Self::listen(socket, config, shutdown)
    }

    fn listen(
        socket: socket2::Socket,
        config: ListenerConfig,
        shutdown: CancellationToken,
    ) -> Result<Self, TransportError> {
        let inner = into_tokio_listener(socket)?;
        let local_addr = inner.local_addr().map_err(TransportError::SocketConfig)?;

        let semaphore = if config.max_connections > 0 {
            Some(Arc::new(Semaphore::new(config.max_connections as usize)))
        } else {
            None
        };

        tracing::info!(
            bind = %local_addr,
            max_connections = config.max_connections,
            max_connections_per_ip = config.max_connections_per_ip,
            "listener bound"
        );

        Ok(Self {
            inner,
            local_addr,
            semaphore,
            ip_limiter: IpLimiter::new(config.max_connections_per_ip),
            config,
            shutdown,
        })
    }

    pub async fn accept(&self) -> Result<PendingConnection, TransportError> {
        let mut backoff = std::time::Duration::from_millis(100);
        loop {
            let permit = if let Some(sem) = &self.semaphore {
                let permit = tokio::select! {
                    biased;
                    () = self.shutdown.cancelled() => {
                        return Err(TransportError::Shutdown);
                    }
                    result = sem.clone().acquire_owned() => {
                        result.map_err(|_| TransportError::Shutdown)?
                    }
                };
                Some(permit)
            } else {
                None
            };

            let (stream, peer_addr) = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => {
                    return Err(TransportError::Shutdown);
                }
                result = self.inner.accept() => {
                    match result {
                        Ok((stream, addr)) => (stream, addr),
                        Err(e) if is_transient_error(&e) => {
                            tracing::warn!(error = %e, backoff_ms = backoff.as_millis(), "transient accept error, retrying");
                            sleep_unless_cancelled(backoff, &self.shutdown).await?;
                            backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
                            continue;
                        }
                        Err(e) => return Err(TransportError::Accept(e)),
                    }
                }
            };

            configure_tokio_stream(&stream, &self.config.keepalive)?;

            return Ok(PendingConnection {
                connection: ClientConnection::new(stream, peer_addr, self.local_addr),
                proxy_header: ProxyHeader::for_peer(&self.config, peer_addr.ip()),
                ip_limiter: self.ip_limiter.clone(),
                permit,
            });
        }
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

async fn sleep_unless_cancelled(
    delay: std::time::Duration,
    shutdown: &CancellationToken,
) -> Result<(), TransportError> {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => Err(TransportError::Shutdown),
        () = tokio::time::sleep(delay) => Ok(()),
    }
}

fn is_transient_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::OutOfMemory
    ) || e
        .raw_os_error()
        .is_some_and(|errno| RESOURCE_EXHAUSTION.contains(&errno))
}

#[cfg(target_os = "linux")]
const RESOURCE_EXHAUSTION: [i32; 4] = {
    use nix::errno::Errno;
    [
        Errno::EMFILE as i32,
        Errno::ENFILE as i32,
        Errno::ENOMEM as i32,
        Errno::ENOBUFS as i32,
    ]
};

#[cfg(target_os = "macos")]
const RESOURCE_EXHAUSTION: [i32; 4] = {
    const EMFILE: i32 = 24;
    const ENFILE: i32 = 23;
    const ENOMEM: i32 = 12;
    const ENOBUFS: i32 = 55;
    [EMFILE, ENFILE, ENOMEM, ENOBUFS]
};

#[cfg(windows)]
const RESOURCE_EXHAUSTION: [i32; 2] = {
    const WSAEMFILE: i32 = 10024;
    const WSAENOBUFS: i32 = 10055;
    [WSAEMFILE, WSAENOBUFS]
};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
const RESOURCE_EXHAUSTION: [i32; 0] = [];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn proxy_config(trusted: &[&str]) -> ListenerConfig {
        ListenerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            max_connections: 0,
            max_connections_per_ip: 0,
            keepalive: KeepaliveConfig::default(),
            so_reuseport: false,
            receive_proxy_protocol: true,
            trusted_proxies: trusted.iter().map(|net| net.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn an_ipv4_mapped_peer_matches_ipv4_trusted_networks() {
        let config = proxy_config(&["10.0.0.0/8"]);
        let mapped: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        assert!(matches!(
            ProxyHeader::for_peer(&config, mapped),
            ProxyHeader::Expected
        ));
    }

    #[test]
    fn an_ipv4_mapped_peer_does_not_match_ipv6_networks() {
        let config = proxy_config(&["::/0"]);
        let mapped: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        assert!(matches!(
            ProxyHeader::for_peer(&config, mapped),
            ProxyHeader::UntrustedPeer
        ));
    }

    #[test]
    fn an_empty_trust_list_refuses_every_peer() {
        let config = proxy_config(&[]);
        assert!(matches!(
            ProxyHeader::for_peer(&config, "127.0.0.1".parse().unwrap()),
            ProxyHeader::UntrustedPeer
        ));
    }

    #[test]
    fn transient_errors_are_retried() {
        for errno in RESOURCE_EXHAUSTION {
            assert!(
                is_transient_error(&std::io::Error::from_raw_os_error(errno)),
                "errno {errno} should be transient"
            );
        }
        for kind in [
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
        ] {
            assert!(is_transient_error(&std::io::Error::from(kind)));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_accept_backoff_ends_as_soon_as_shutdown_is_requested() {
        let shutdown = CancellationToken::new();
        let started = tokio::time::Instant::now();
        let backoff = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { sleep_unless_cancelled(std::time::Duration::from_secs(5), &shutdown).await }
        });
        tokio::task::yield_now().await;

        shutdown.cancel();

        assert!(matches!(
            backoff.await.unwrap(),
            Err(TransportError::Shutdown)
        ));
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn the_accept_backoff_lasts_its_full_delay_without_shutdown() {
        let started = tokio::time::Instant::now();
        let delay = std::time::Duration::from_secs(5);

        sleep_unless_cancelled(delay, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(started.elapsed(), delay);
    }

    #[test]
    fn fatal_errors_are_not_retried() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
        ] {
            assert!(!is_transient_error(&std::io::Error::from(kind)));
        }
    }
}
