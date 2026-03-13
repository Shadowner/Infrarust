//! Bidirectional TCP forwarding.
//!
//! Provides two forwarding strategies:
//! - `CopyForwarder`: portable userspace copy via `tokio::io::copy_bidirectional`
//! - `SpliceForwarder`: Linux-only zero-copy via `splice(2)` syscall

use std::future::Future;
use std::pin::Pin;

use tokio::net::TcpStream;
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
    Error(std::io::Error),
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

/// Portable userspace copy forwarder using `tokio::io::copy_bidirectional`.
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
    mut client: TcpStream,
    mut backend: TcpStream,
    shutdown: CancellationToken,
) -> ForwardResult {
    tokio::select! {
        result = tokio::io::copy_bidirectional(&mut client, &mut backend) => {
            match result {
                Ok((c2b, b2c)) => ForwardResult {
                    client_to_backend: c2b,
                    backend_to_client: b2c,
                    reason: ForwardEndReason::ClientClosed,
                },
                Err(e) => ForwardResult {
                    client_to_backend: 0,
                    backend_to_client: 0,
                    reason: ForwardEndReason::Error(e),
                },
            }
        }
        () = shutdown.cancelled() => {
            ForwardResult {
                client_to_backend: 0,
                backend_to_client: 0,
                reason: ForwardEndReason::Shutdown,
            }
        }
    }
}

// ─────────────────────── Linux splice forwarder ──────────────────────

#[cfg(target_os = "linux")]
mod splice_impl {
    use super::{
        CancellationToken, ForwardEndReason, ForwardResult, Forwarder, Future, Pin, TcpStream,
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
    }

    impl KernelPipe {
        fn new(size: usize) -> std::io::Result<Self> {
            let (read_fd, write_fd) = nix::unistd::pipe().map_err(std::io::Error::other)?;

            // Set nonblocking
            for fd in [read_fd.as_raw_fd(), write_fd.as_raw_fd()] {
                nix::fcntl::fcntl(
                    fd,
                    nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
                )
                .map_err(std::io::Error::other)?;
            }

            // Try to set pipe size
            // Pipe size is always a reasonable value (e.g. 64 KiB), safe to truncate to i32.
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let _ = nix::fcntl::fcntl(
                write_fd.as_raw_fd(),
                nix::fcntl::FcntlArg::F_SETPIPE_SZ(size as i32),
            );

            Ok(Self { read_fd, write_fd })
        }
    }

    /// Splices data in one direction via a kernel pipe.
    ///
    /// Uses the `TcpStream`'s readiness methods to avoid double epoll registration.
    async fn splice_one_direction(
        src: &TcpStream,
        dst: &TcpStream,
        pipe: &KernelPipe,
        shutdown: CancellationToken,
    ) -> Result<u64, std::io::Error> {
        use nix::fcntl::SpliceFFlags;
        use tokio::io::Interest;

        let mut total: u64 = 0;
        let flags = SpliceFFlags::SPLICE_F_NONBLOCK | SpliceFFlags::SPLICE_F_MOVE;

        loop {
            // Drain: source → pipe
            let drained = tokio::select! {
                result = src.ready(Interest::READABLE) => {
                    let _ = result?;
                    let src_fd = unsafe { BorrowedFd::borrow_raw(src.as_raw_fd()) };
                    match src.try_io(Interest::READABLE, || {
                        nix::fcntl::splice(
                            src_fd,
                            None,
                            &pipe.write_fd,
                            None,
                            65536,
                            flags,
                        ).map_err(std::io::Error::other)
                    }) {
                        Ok(0) => break, // EOF
                        Ok(n) => n,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(e) => return Err(e),
                    }
                }
                () = shutdown.cancelled() => break,
            };

            // Pump: pipe → destination
            let mut pumped = 0usize;
            while pumped < drained {
                tokio::select! {
                    result = dst.ready(Interest::WRITABLE) => {
                        let _ = result?;
                        let dst_fd = unsafe { BorrowedFd::borrow_raw(dst.as_raw_fd()) };
                        match dst.try_io(Interest::WRITABLE, || {
                            nix::fcntl::splice(
                                &pipe.read_fd,
                                None,
                                dst_fd,
                                None,
                                drained - pumped,
                                flags,
                            ).map_err(std::io::Error::other)
                        }) {
                            Ok(0) => return Err(std::io::Error::new(
                                std::io::ErrorKind::WriteZero,
                                "splice pump returned 0"
                            )),
                            Ok(n) => pumped += n,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
                            Err(e) => return Err(e),
                        }
                    }
                    () = shutdown.cancelled() => break,
                }
            }

            // pumped is a byte count from splice, always fits in u64.
            #[allow(clippy::cast_possible_truncation)]
            {
                total += pumped as u64;
            }
        }

        Ok(total)
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

                let (mut client_to_backend, mut backend_to_client) = (0u64, 0u64);

                let reason = tokio::select! {
                    result = &mut c2b => {
                        match result {
                            Ok(bytes) => {
                                client_to_backend = bytes;
                                shutdown.cancel();
                                if let Ok(bytes) = (&mut b2c).await {
                                    backend_to_client = bytes;
                                }
                                ForwardEndReason::ClientClosed
                            }
                            Err(e) => ForwardEndReason::Error(e),
                        }
                    }
                    result = &mut b2c => {
                        match result {
                            Ok(bytes) => {
                                backend_to_client = bytes;
                                shutdown.cancel();
                                if let Ok(bytes) = (&mut c2b).await {
                                    client_to_backend = bytes;
                                }
                                ForwardEndReason::BackendClosed
                            }
                            Err(e) => ForwardEndReason::Error(e),
                        }
                    }
                    () = shutdown.cancelled() => {
                        ForwardEndReason::Shutdown
                    }
                };

                // client and backend are kept alive by the borrow in c2b/b2c
                // and will be dropped when this future completes.
                let _ = (&client, &backend);

                ForwardResult {
                    client_to_backend,
                    backend_to_client,
                    reason,
                }
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
