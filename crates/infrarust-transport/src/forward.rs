//! Bidirectional TCP forwarding.
//!
//! Provides two forwarding strategies:
//! - `CopyForwarder`: portable userspace copy via two spawned `tokio::io::copy` tasks
//! - `SpliceForwarder`: Linux-only zero-copy via `splice(2)` syscall

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;

use infrarust_config::ProxyMode;

/// Result of a forwarding session.
#[derive(Debug)]
pub struct ForwardResult {
    /// Bytes transferred from client to backend.
    pub client_to_backend: u64,
    /// Bytes transferred from backend to client.
    pub backend_to_client: u64,
    /// Reason the forwarding ended.
    pub reason: ForwardEndReason,
}

/// Reason a forwarding session ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum ForwardEndReason {
    /// Client closed the connection.
    ClientClosed,
    /// Backend closed the connection.
    BackendClosed,
    /// Shutdown signal received.
    Shutdown,
    /// I/O error during forwarding.
    Error(io::Error),
}

/// Trait for bidirectional TCP forwarding strategies.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility.
pub trait Forwarder: Send + Sync {
    /// Forwards data bidirectionally between client and backend.
    fn forward(
        &self,
        client: TcpStream,
        backend: TcpStream,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = ForwardResult> + Send + '_>>;
}

/// Portable userspace copy forwarder spawning one `tokio::io::copy` task per direction.
#[derive(Debug, Default)]
pub struct CopyForwarder;

impl Forwarder for CopyForwarder {
    fn forward(
        &self,
        client: TcpStream,
        backend: TcpStream,
        shutdown: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = ForwardResult> + Send + '_>> {
        Box::pin(copy_forward(client, backend, shutdown))
    }
}

async fn copy_forward(
    client: TcpStream,
    backend: TcpStream,
    shutdown: CancellationToken,
) -> ForwardResult {
    let (client_read, client_write) = client.into_split();
    let (backend_read, backend_write) = backend.into_split();
    let first = Arc::new(OnceLock::new());

    let mut c2b = tokio::spawn(copy_until_eof(
        client_read,
        backend_write,
        Arc::clone(&first),
        Side::Client,
    ));
    let mut b2c = tokio::spawn(copy_until_eof(
        backend_read,
        client_write,
        Arc::clone(&first),
        Side::Backend,
    ));

    let mut c2b_end = None;
    let mut b2c_end = None;
    while c2b_end.is_none() || b2c_end.is_none() {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            ended = &mut c2b, if c2b_end.is_none() => c2b_end = Some(joined(ended)),
            ended = &mut b2c, if b2c_end.is_none() => b2c_end = Some(joined(ended)),
        }
        if matches!(c2b_end, Some(Err(_))) || matches!(b2c_end, Some(Err(_))) {
            break;
        }
    }

    c2b.abort();
    b2c.abort();
    if c2b_end.is_none() {
        c2b_end = c2b.await.ok();
    }
    if b2c_end.is_none() {
        b2c_end = b2c.await.ok();
    }
    finish(first.get().copied(), c2b_end, b2c_end)
}

async fn copy_until_eof(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    first: Arc<OnceLock<Side>>,
    side: Side,
) -> io::Result<u64> {
    use tokio::io::AsyncWriteExt;

    let copied = tokio::io::copy(&mut from, &mut to).await;
    let _ = first.set(side);
    let _ = to.shutdown().await;
    copied
}

fn joined(result: Result<io::Result<u64>, JoinError>) -> io::Result<u64> {
    result.unwrap_or_else(|e| Err(io::Error::other(e)))
}

#[derive(Debug, Clone, Copy)]
enum Side {
    Client,
    Backend,
}

fn finish(
    first: Option<Side>,
    c2b: Option<io::Result<u64>>,
    b2c: Option<io::Result<u64>>,
) -> ForwardResult {
    let client_to_backend = transferred(c2b.as_ref());
    let backend_to_client = transferred(b2c.as_ref());
    let reason = match first {
        Some(Side::Client) => blame(c2b, ForwardEndReason::ClientClosed),
        Some(Side::Backend) => blame(b2c, ForwardEndReason::BackendClosed),
        None => match (c2b, b2c) {
            (Some(Err(e)), _) | (_, Some(Err(e))) => ForwardEndReason::Error(e),
            _ => ForwardEndReason::Shutdown,
        },
    };
    ForwardResult {
        client_to_backend,
        backend_to_client,
        reason,
    }
}

fn blame(end: Option<io::Result<u64>>, closed: ForwardEndReason) -> ForwardEndReason {
    match end {
        Some(Err(e)) => ForwardEndReason::Error(e),
        _ => closed,
    }
}

fn transferred(end: Option<&io::Result<u64>>) -> u64 {
    end.and_then(|result| result.as_ref().ok())
        .copied()
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
mod splice_impl {
    use super::{
        CancellationToken, ForwardEndReason, ForwardResult, Forwarder, Future, Pin, Side,
        TcpStream, finish,
    };
    use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};

    /// Zero-copy forwarder using the Linux `splice(2)` syscall.
    #[derive(Debug)]
    pub struct SpliceForwarder {
        pipe_size: usize,
    }

    impl Default for SpliceForwarder {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SpliceForwarder {
        /// Creates a new splice forwarder with default pipe size (64 KiB).
        pub const fn new() -> Self {
            Self {
                pipe_size: 64 * 1024,
            }
        }

        /// Creates a new splice forwarder with custom pipe size.
        pub const fn with_pipe_size(pipe_size: usize) -> Self {
            Self { pipe_size }
        }
    }

    struct KernelPipe {
        read_fd: OwnedFd,
        write_fd: OwnedFd,
        size: usize,
    }

    impl KernelPipe {
        fn new(size: usize) -> std::io::Result<Self> {
            let (read_fd, write_fd) = nix::unistd::pipe().map_err(std::io::Error::other)?;

            // Set nonblocking
            for fd in [&read_fd, &write_fd] {
                nix::fcntl::fcntl(
                    fd,
                    nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
                )
                .map_err(std::io::Error::other)?;
            }

            // Try to set pipe size
            // Pipe size is always a reasonable value (e.g. 64 KiB), safe to truncate to i32.
            let _ = nix::fcntl::fcntl(&write_fd, nix::fcntl::FcntlArg::F_SETPIPE_SZ(size as i32));

            Ok(Self {
                read_fd,
                write_fd,
                size,
            })
        }
    }

    /// Splices data in one direction via a kernel pipe.
    ///
    /// Uses the `TcpStream`'s readiness methods to avoid double epoll registration.
    /// When the source reaches EOF, the destination's write half is shut down to
    /// propagate the EOF signal to the other direction.
    /// Converts a nix errno to a `std::io::Error`, preserving the OS error code.
    ///
    /// Unlike `Error::other()`, this correctly maps EAGAIN to `ErrorKind::WouldBlock`,
    /// which is required for `try_io`'s readiness-clearing contract.
    fn nix_to_io_error(e: nix::Error) -> std::io::Error {
        std::io::Error::from_raw_os_error(e as i32)
    }

    async fn splice_one_direction(
        src: &TcpStream,
        dst: &TcpStream,
        pipe: &KernelPipe,
        shutdown: CancellationToken,
    ) -> Result<Option<u64>, std::io::Error> {
        use nix::fcntl::SpliceFFlags;
        use tokio::io::Interest;

        let mut total: u64 = 0;
        let flags = SpliceFFlags::SPLICE_F_NONBLOCK | SpliceFFlags::SPLICE_F_MOVE;
        let mut eof_received = false;

        loop {
            // Drain: source → pipe
            let drained = tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                result = src.ready(Interest::READABLE) => {
                    let _ = result?;
                    // SAFETY: `TcpStream` owns a valid file descriptor for its entire lifetime.
                    // We borrow it only within the scope of `try_io`'s closure, which guarantees
                    // the fd remains valid. The `TcpStream` is not dropped or moved during this call.
                    #[allow(unsafe_code)]
                    let src_fd = unsafe { BorrowedFd::borrow_raw(src.as_raw_fd()) };
                    match src.try_io(Interest::READABLE, || {
                        nix::fcntl::splice(
                            src_fd,
                            None,
                            &pipe.write_fd,
                            None,
                            pipe.size,
                            flags,
                        ).map_err(nix_to_io_error)
                    }) {
                        Ok(0) => { eof_received = true; break; }
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e),
                    }
                }
            };

            // Pump: pipe → destination
            let mut pumped = 0usize;
            while pumped < drained {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        return Ok(None);
                    }
                    result = dst.ready(Interest::WRITABLE) => {
                        let _ = result?;
                        // SAFETY: `TcpStream` owns a valid file descriptor for its entire lifetime.
                        // We borrow it only within the scope of `try_io`'s closure, which guarantees
                        // the fd remains valid. The `TcpStream` is not dropped or moved during this call.
                        #[allow(unsafe_code)]
                        let dst_fd = unsafe { BorrowedFd::borrow_raw(dst.as_raw_fd()) };
                        match dst.try_io(Interest::WRITABLE, || {
                            nix::fcntl::splice(&pipe.read_fd, None, dst_fd, None, drained - pumped, flags)
                                .map_err(nix_to_io_error)
                        }) {
                            Ok(0) => {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::WriteZero,
                                    "splice pump returned 0",
                                ));
                            }
                            Ok(n) => pumped += n,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
            }

            // pumped is a byte count from splice, always fits in u64.
            total += pumped as u64;
        }

        // Propagate EOF to the other end by half-closing the destination's write side.
        // This lets the other direction detect EOF and finish naturally.
        if eof_received {
            let _ = nix::sys::socket::shutdown(dst.as_raw_fd(), nix::sys::socket::Shutdown::Write);
        }

        Ok(eof_received.then_some(total))
    }

    impl Forwarder for SpliceForwarder {
        fn forward(
            &self,
            client: TcpStream,
            backend: TcpStream,
            shutdown: CancellationToken,
        ) -> Pin<Box<dyn Future<Output = ForwardResult> + Send + '_>> {
            let pipe_size = self.pipe_size;
            Box::pin(async move {
                let pipe_c2b = match KernelPipe::new(pipe_size) {
                    Ok(p) => p,
                    Err(e) => {
                        return ForwardResult {
                            client_to_backend: 0,
                            backend_to_client: 0,
                            reason: ForwardEndReason::Error(e),
                        };
                    }
                };

                let pipe_b2c = match KernelPipe::new(pipe_size) {
                    Ok(p) => p,
                    Err(e) => {
                        return ForwardResult {
                            client_to_backend: 0,
                            backend_to_client: 0,
                            reason: ForwardEndReason::Error(e),
                        };
                    }
                };

                let c2b = splice_one_direction(&client, &backend, &pipe_c2b, shutdown.clone());
                let b2c = splice_one_direction(&backend, &client, &pipe_b2c, shutdown.clone());

                tokio::pin!(c2b);
                tokio::pin!(b2c);

                let mut first = None;
                let mut c2b_end = None;
                let mut b2c_end = None;
                while c2b_end.is_none() || b2c_end.is_none() {
                    let (side, ended) = tokio::select! {
                        biased;
                        () = shutdown.cancelled() => break,
                        ended = &mut c2b, if c2b_end.is_none() => (Side::Client, ended),
                        ended = &mut b2c, if b2c_end.is_none() => (Side::Backend, ended),
                    };
                    let Some(ended) = ended.transpose() else {
                        break;
                    };
                    let failed = ended.is_err();
                    first.get_or_insert(side);
                    match side {
                        Side::Client => c2b_end = Some(ended),
                        Side::Backend => b2c_end = Some(ended),
                    }
                    if failed {
                        break;
                    }
                }
                finish(first, c2b_end, b2c_end)
            })
        }
    }
}

#[cfg(target_os = "linux")]
pub use splice_impl::SpliceForwarder;

/// Selects the appropriate forwarder based on the proxy mode.
pub fn select_forwarder(mode: ProxyMode) -> Box<dyn Forwarder> {
    match mode {
        ProxyMode::ZeroCopy => {
            #[cfg(target_os = "linux")]
            {
                Box::new(SpliceForwarder::new())
            }
            #[cfg(not(target_os = "linux"))]
            {
                tracing::warn!(
                    "ZeroCopy mode requested but splice is only available on Linux, \
                     falling back to CopyForwarder"
                );
                Box::new(CopyForwarder)
            }
        }
        _ => Box::new(CopyForwarder),
    }
}
