use std::net::SocketAddr;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use listenfd::ListenFd;
use socket2::{Domain, Protocol, SockRef, Socket, TcpKeepalive, Type};
use tokio::net::{TcpListener, TcpStream};

use infrarust_config::KeepaliveConfig;

use crate::error::TransportError;

pub fn activated_listener() -> Result<Option<std::net::TcpListener>, TransportError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        ListenFd::from_env()
            .take_tcp_listener(0)
            .map_err(TransportError::SocketActivation)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(None)
    }
}

pub(crate) fn adopt_listener_socket(
    listener: std::net::TcpListener,
) -> Result<Socket, TransportError> {
    let socket = Socket::from(listener);
    socket
        .set_nonblocking(true)
        .map_err(TransportError::SocketConfig)?;
    Ok(socket)
}

pub(crate) fn configure_listener_socket(
    addr: SocketAddr,
    reuseport: bool,
) -> Result<Socket, TransportError> {
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

fn configure_stream_socket(
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

pub(crate) fn configure_tokio_stream(
    stream: &TcpStream,
    keepalive: &KeepaliveConfig,
) -> Result<(), TransportError> {
    configure_stream_socket(&SockRef::from(stream), keepalive)
}

pub(crate) fn into_tokio_listener(socket: Socket) -> Result<TcpListener, TransportError> {
    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener).map_err(TransportError::SocketConfig)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
