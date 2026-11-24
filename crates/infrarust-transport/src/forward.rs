use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bytes::Bytes;

use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;

use infrarust_config::ProxyMode;

#[derive(Debug)]
pub struct ForwardResult {
    pub client_to_backend: u64,
    pub backend_to_client: u64,
    pub reason: ForwardEndReason,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ForwardEndReason {
    ClientClosed,
    BackendClosed,
    Shutdown,
    Frozen(Box<FrozenRelay>),
    Error(io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaySide {
    Client,
    Backend,
}

#[derive(Debug, Default)]
pub struct FrozenDirection {
    pub tail: Bytes,
    pub source_eof: bool,
    pub copied: u64,
}

#[derive(Debug)]
pub struct FrozenRelay {
    pub client: std::net::TcpStream,
    pub backend: std::net::TcpStream,
    pub client_to_backend: FrozenDirection,
    pub backend_to_client: FrozenDirection,
    pub first_closed: Option<RelaySide>,
}

#[derive(Debug, Clone)]
pub struct Freeze {
    token: CancellationToken,
    write_deadline: Duration,
}

impl Freeze {
    pub const DEFAULT_WRITE_DEADLINE: Duration = Duration::from_millis(100);

    pub const fn new(token: CancellationToken, write_deadline: Duration) -> Self {
        Self {
            token,
            write_deadline,
        }
    }

    pub fn never() -> Self {
        Self::new(CancellationToken::new(), Self::DEFAULT_WRITE_DEADLINE)
    }

    pub const fn token(&self) -> &CancellationToken {
        &self.token
    }

    pub const fn write_deadline(&self) -> Duration {
        self.write_deadline
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forwarder {
    Copy,
    #[cfg(target_os = "linux")]
    Splice,
}

impl Forwarder {
    pub fn for_mode(mode: ProxyMode) -> Self {
        if mode != ProxyMode::ZeroCopy {
            return Self::Copy;
        }
        #[cfg(target_os = "linux")]
        {
            Self::Splice
        }
        #[cfg(not(target_os = "linux"))]
        {
            tracing::warn!(
                "ZeroCopy mode requested but splice is only available on Linux, \
                 falling back to userspace copy"
            );
            Self::Copy
        }
    }

    pub async fn forward(
        self,
        client: TcpStream,
        backend: TcpStream,
        shutdown: CancellationToken,
        _freeze: Freeze,
    ) -> ForwardResult {
        match self {
            Self::Copy => copy_forward(client, backend, shutdown).await,
            #[cfg(target_os = "linux")]
            Self::Splice => splice_impl::splice_forward(client, backend, shutdown).await,
        }
    }

    pub async fn resume(
        self,
        relay: FrozenRelay,
        shutdown: CancellationToken,
        freeze: Freeze,
    ) -> ForwardResult {
        use tokio::io::AsyncWriteExt;

        let streams = adopt(relay.client).and_then(|client| Ok((client, adopt(relay.backend)?)));
        let (mut client, mut backend) = match streams {
            Ok(streams) => streams,
            Err(e) => {
                return ForwardResult {
                    client_to_backend: 0,
                    backend_to_client: 0,
                    reason: ForwardEndReason::Error(e),
                };
            }
        };
        for (to, tail) in [
            (&mut backend, &relay.client_to_backend.tail),
            (&mut client, &relay.backend_to_client.tail),
        ] {
            if let Err(e) = to.write_all(tail).await {
                return ForwardResult {
                    client_to_backend: 0,
                    backend_to_client: 0,
                    reason: ForwardEndReason::Error(e),
                };
            }
        }
        self.forward(client, backend, shutdown, freeze).await
    }
}

fn adopt(stream: std::net::TcpStream) -> io::Result<TcpStream> {
    stream.set_nonblocking(true)?;
    TcpStream::from_std(stream)
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
    use super::{CancellationToken, ForwardEndReason, ForwardResult, Side, TcpStream, finish};
    use std::os::fd::{AsFd, OwnedFd};

    const PIPE_SIZE: usize = 64 * 1024;

    struct KernelPipe {
        read_fd: OwnedFd,
        write_fd: OwnedFd,
        size: usize,
    }

    impl KernelPipe {
        fn new(requested: usize) -> std::io::Result<Self> {
            use nix::fcntl::{FcntlArg, OFlag, fcntl};

            static UNDERSIZED: std::sync::Once = std::sync::Once::new();

            let (read_fd, write_fd) = nix::unistd::pipe2(OFlag::O_NONBLOCK | OFlag::O_CLOEXEC)
                .map_err(nix_to_io_error)?;

            let _ = fcntl(
                &write_fd,
                FcntlArg::F_SETPIPE_SZ(i32::try_from(requested).unwrap_or(i32::MAX)),
            );
            let size = fcntl(&write_fd, FcntlArg::F_GETPIPE_SZ).map_err(nix_to_io_error)?;
            let size = usize::try_from(size).map_err(std::io::Error::other)?;
            if size < requested {
                UNDERSIZED.call_once(|| {
                    tracing::warn!(
                        requested,
                        actual = size,
                        "splice pipe is smaller than requested"
                    );
                });
            }

            Ok(Self {
                read_fd,
                write_fd,
                size,
            })
        }
    }

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
        let cancelled = shutdown.cancelled();
        tokio::pin!(cancelled);

        loop {
            let drained = tokio::select! {
                biased;
                () = &mut cancelled => break,
                result = src.ready(Interest::READABLE) => {
                    let _ = result?;
                    match src.try_io(Interest::READABLE, || {
                        nix::fcntl::splice(
                            src.as_fd(),
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

            let mut pumped = 0usize;
            while pumped < drained {
                tokio::select! {
                    biased;
                    () = &mut cancelled => {
                        return Ok(None);
                    }
                    result = dst.ready(Interest::WRITABLE) => {
                        let _ = result?;
                        match dst.try_io(Interest::WRITABLE, || {
                            nix::fcntl::splice(&pipe.read_fd, None, dst.as_fd(), None, drained - pumped, flags)
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

            total += pumped as u64;
        }

        if eof_received {
            let _ = socket2::SockRef::from(dst).shutdown(std::net::Shutdown::Write);
        }

        Ok(eof_received.then_some(total))
    }

    fn pipe_failed(e: std::io::Error) -> ForwardResult {
        ForwardResult {
            client_to_backend: 0,
            backend_to_client: 0,
            reason: ForwardEndReason::Error(e),
        }
    }

    pub(super) async fn splice_forward(
        client: TcpStream,
        backend: TcpStream,
        shutdown: CancellationToken,
    ) -> ForwardResult {
        let pipe_c2b = match KernelPipe::new(PIPE_SIZE) {
            Ok(pipe) => pipe,
            Err(e) => return pipe_failed(e),
        };
        let pipe_b2c = match KernelPipe::new(PIPE_SIZE) {
            Ok(pipe) => pipe,
            Err(e) => return pipe_failed(e),
        };

        let c2b = splice_one_direction(&client, &backend, &pipe_c2b, shutdown.clone());
        let b2c = splice_one_direction(&backend, &client, &pipe_b2c, shutdown.clone());

        let cancelled = shutdown.cancelled();
        tokio::pin!(c2b, b2c, cancelled);

        let mut first = None;
        let mut c2b_end = None;
        let mut b2c_end = None;
        while c2b_end.is_none() || b2c_end.is_none() {
            let (side, ended) = tokio::select! {
                biased;
                () = &mut cancelled => break,
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
    }
}
