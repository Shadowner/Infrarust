//! Socket configuration via `socket2`.
//!
//! Provides functions to create and configure TCP sockets with
//! `SO_REUSEADDR`, `SO_REUSEPORT`, `TCP_NODELAY`, and TCP keepalive
//! before converting them to tokio types.

use std::net::SocketAddr;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use listenfd::ListenFd;
use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use tokio::net::{TcpListener, TcpStream};

use infrarust_config::KeepaliveConfig;

use crate::error::TransportError;

#[cfg(any(target_os = "linux", target_os = "macos"))]
static ACTIVATED_LISTENER: std::sync::Mutex<Option<std::net::TcpListener>> =
    std::sync::Mutex::new(None);

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn activated_listener() -> std::sync::MutexGuard<'static, Option<std::net::TcpListener>> {
    ACTIVATED_LISTENER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn init_socket_activation() -> Result<(), TransportError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if let Some(listener) = ListenFd::from_env()
            .take_tcp_listener(0)
            .map_err(TransportError::SocketActivation)?
        {
            *activated_listener() = Some(listener);
        }
    }

    Ok(())
}

/// Creates and configures a listener socket.
///
/// Sets `SO_REUSEADDR` (always), `SO_REUSEPORT` (Linux, if requested),
/// nonblocking mode, then binds and listens with a backlog of 1024.
///
/// If [`init_socket_activation`] claimed a socket, adopts that socket and
/// only sets nonblocking mode on it. `addr` and `reuseport` are then
/// ignored: the listening address, backlog and socket options belong to
/// whatever activated the socket.
///
/// # Errors
///
/// Returns [`TransportError::SocketConfig`] if socket creation or
/// configuration fails, or [`TransportError::Bind`] if binding fails.
pub fn configure_listener_socket(
    addr: SocketAddr,
    reuseport: bool,
) -> Result<Socket, TransportError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(listener) = activated_listener().take() {
        let socket = Socket::from(listener);
        socket
            .set_nonblocking(true)
            .map_err(TransportError::SocketConfig)?;

        tracing::info!(configured_bind = %addr, "adopting socket passed by socket activation");
        return Ok(socket);
    }

    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };

    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
        .map_err(TransportError::SocketConfig)?;

    socket
        .set_reuse_address(true)
        .map_err(TransportError::SocketConfig)?;

    #[cfg(target_os = "linux")]
    if reuseport {
        socket
            .set_reuse_port(true)
            .map_err(TransportError::SocketConfig)?;
    }

    #[cfg(not(target_os = "linux"))]
    let _ = reuseport;

    socket
        .set_nonblocking(true)
        .map_err(TransportError::SocketConfig)?;

    socket
        .bind(&addr.into())
        .map_err(|source| TransportError::Bind { addr, source })?;

    socket
        .listen(1024)
        .map_err(|source| TransportError::Bind { addr, source })?;

    Ok(socket)
}

/// Configures a stream socket with `TCP_NODELAY` and keepalive.
///
/// # Errors
///
/// Returns [`TransportError::SocketConfig`] if setting `TCP_NODELAY` or
/// keepalive options fails.
pub fn configure_stream_socket(
    socket: &Socket,
    keepalive: &KeepaliveConfig,
) -> Result<(), TransportError> {
    socket
        .set_tcp_nodelay(true)
        .map_err(TransportError::SocketConfig)?;

    let mut ka = TcpKeepalive::new()
        .with_time(keepalive.time)
        .with_interval(keepalive.interval);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        ka = ka.with_retries(keepalive.retries);
    }

    socket
        .set_tcp_keepalive(&ka)
        .map_err(TransportError::SocketConfig)?;

    Ok(())
}

/// Converts a `socket2::Socket` into a `tokio::net::TcpListener`.
///
/// The socket must already be in nonblocking mode and bound+listening.
///
/// # Errors
///
/// Returns [`TransportError::SocketConfig`] if the conversion fails.
pub fn into_tokio_listener(socket: Socket) -> Result<TcpListener, TransportError> {
    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener).map_err(TransportError::SocketConfig)
}

/// Converts a `socket2::Socket` into a `tokio::net::TcpStream`.
///
/// The socket must already be in nonblocking mode.
///
/// # Errors
///
/// Returns [`TransportError::SocketConfig`] if the conversion fails.
pub fn into_tokio_stream(socket: Socket) -> Result<TcpStream, TransportError> {
    let std_stream: std::net::TcpStream = socket.into();
    TcpStream::from_std(std_stream).map_err(TransportError::SocketConfig)
}
