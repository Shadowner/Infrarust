use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    budget: Option<Arc<ConnectionBudget>>,
    ip_limiter: Option<Arc<IpLimiter>>,
    config: ListenerConfig,
    shutdown: CancellationToken,
}

#[derive(Debug)]
struct ConnectionBudget {
    semaphore: Arc<Semaphore>,
    debt: Mutex<usize>,
}

impl ConnectionBudget {
    fn new(max_connections: u32) -> Option<Arc<Self>> {
        (max_connections > 0).then(|| {
            Arc::new(Self {
                semaphore: Arc::new(Semaphore::new(max_connections as usize)),
                debt: Mutex::new(0),
            })
        })
    }

    fn debt(&self) -> MutexGuard<'_, usize> {
        self.debt.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn acquire(self: &Arc<Self>) -> Result<Seat, TransportError> {
        let permit = Arc::clone(&self.semaphore)
            .acquire_owned()
            .await
            .map_err(|_| TransportError::Shutdown)?;
        Ok(Seat {
            budget: Arc::clone(self),
            permit: Some(permit),
        })
    }

    fn reserve(self: &Arc<Self>) -> Seat {
        let mut debt = self.debt();
        let permit = Arc::clone(&self.semaphore).try_acquire_owned().ok();
        if permit.is_none() {
            *debt += 1;
        }
        Seat {
            budget: Arc::clone(self),
            permit,
        }
    }
}

#[derive(Debug)]
struct Seat {
    budget: Arc<ConnectionBudget>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for Seat {
    fn drop(&mut self) {
        let mut debt = self.budget.debt();
        match (self.permit.take(), *debt > 0) {
            (Some(permit), true) => {
                *debt -= 1;
                permit.forget();
            }
            (None, true) => *debt -= 1,
            (Some(permit), false) => drop(permit),
            (None, false) => self.budget.semaphore.add_permits(1),
        }
    }
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
    seat: Option<Seat>,
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
            seat,
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
                _seat: seat,
                _ip_guard: ip_guard,
            },
        })
    }
}

#[derive(Debug, Default)]
pub struct ConnectionSlot {
    _seat: Option<Seat>,
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

        tracing::info!(
            bind = %local_addr,
            max_connections = config.max_connections,
            max_connections_per_ip = config.max_connections_per_ip,
            "listener bound"
        );

        Ok(Self {
            inner,
            local_addr,
            budget: ConnectionBudget::new(config.max_connections),
            ip_limiter: IpLimiter::new(config.max_connections_per_ip),
            config,
            shutdown,
        })
    }

    pub async fn accept(&self) -> Result<PendingConnection, TransportError> {
        let mut backoff = std::time::Duration::from_millis(100);
        loop {
            let seat = if let Some(budget) = &self.budget {
                let seat = tokio::select! {
                    biased;
                    () = self.shutdown.cancelled() => {
                        return Err(TransportError::Shutdown);
                    }
                    result = budget.acquire() => result?,
                };
                Some(seat)
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
                seat,
            });
        }
    }

    pub fn reserve(&self, ip: IpAddr) -> ConnectionSlot {
        ConnectionSlot {
            _seat: self.budget.as_ref().map(ConnectionBudget::reserve),
            _ip_guard: self
                .ip_limiter
                .as_ref()
                .map(|limiter| limiter.force_acquire(ip)),
        }
    }

    pub fn into_std(self) -> std::io::Result<std::net::TcpListener> {
        self.inner.into_std()
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

#[cfg(all(unix, not(target_os = "linux")))]
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

#[cfg(not(any(unix, windows)))]
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

    fn state(budget: &ConnectionBudget) -> (usize, usize) {
        (budget.semaphore.available_permits(), *budget.debt())
    }

    #[test]
    fn a_reserve_under_the_limit_takes_a_permit() {
        let budget = ConnectionBudget::new(2).unwrap();
        let restored = budget.reserve();
        assert_eq!(state(&budget), (1, 0));

        drop(restored);
        assert_eq!(state(&budget), (2, 0));
    }

    #[test]
    fn restored_sessions_over_the_limit_are_paid_back_by_the_first_closes() {
        let budget = ConnectionBudget::new(2).unwrap();
        let first = budget.reserve();
        let second = budget.reserve();
        let third = budget.reserve();
        assert!(first.permit.is_some() && second.permit.is_some() && third.permit.is_none());
        assert_eq!(state(&budget), (0, 1));

        drop(first);
        assert_eq!(state(&budget), (0, 0));

        drop(second);
        assert_eq!(state(&budget), (1, 0));

        drop(third);
        assert_eq!(state(&budget), (2, 0));
    }

    #[test]
    fn closing_the_permitless_slot_first_also_pays_the_debt() {
        let budget = ConnectionBudget::new(2).unwrap();
        let first = budget.reserve();
        let second = budget.reserve();
        let third = budget.reserve();

        drop(third);
        assert_eq!(state(&budget), (0, 0));

        drop(first);
        assert_eq!(state(&budget), (1, 0));

        drop(second);
        assert_eq!(state(&budget), (2, 0));
    }

    #[tokio::test]
    async fn accepted_slots_pay_the_debt_of_restored_ones() {
        let budget = ConnectionBudget::new(2).unwrap();
        let first = budget.acquire().await.unwrap();
        let second = budget.acquire().await.unwrap();
        let restored = budget.reserve();
        let over = budget.reserve();
        assert_eq!(state(&budget), (0, 2));

        drop(first);
        assert_eq!(state(&budget), (0, 1));

        drop(restored);
        assert_eq!(state(&budget), (0, 0));

        drop(second);
        assert_eq!(state(&budget), (1, 0));

        let third = budget.acquire().await.unwrap();
        assert_eq!(state(&budget), (0, 0));

        drop(over);
        drop(third);
        assert_eq!(state(&budget), (2, 0));
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
