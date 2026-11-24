use std::io;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use infrarust_config::ProxyMode;

const COPY_BUFFER_SIZE: usize = 8 * 1024;

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
        freeze: Freeze,
    ) -> ForwardResult {
        self.relay(client, backend, Progress::default(), shutdown, freeze)
            .await
    }

    pub async fn resume(
        self,
        relay: FrozenRelay,
        shutdown: CancellationToken,
        freeze: Freeze,
    ) -> ForwardResult {
        let FrozenRelay {
            client,
            backend,
            client_to_backend,
            backend_to_client,
            first_closed,
        } = relay;
        let progress = Progress {
            client_to_backend,
            backend_to_client,
            first_closed,
        };
        let streams = progress
            .check()
            .and_then(|()| adopt(client))
            .and_then(|client| Ok((client, adopt(backend)?)));
        match streams {
            Ok((client, backend)) => {
                self.relay(client, backend, progress, shutdown, freeze)
                    .await
            }
            Err(e) => progress.failed(e),
        }
    }

    async fn relay(
        self,
        client: TcpStream,
        backend: TcpStream,
        progress: Progress,
        shutdown: CancellationToken,
        freeze: Freeze,
    ) -> ForwardResult {
        match self {
            Self::Copy => copy_forward(client, backend, progress, shutdown, freeze).await,
            #[cfg(target_os = "linux")]
            Self::Splice => splice_impl::splice_forward(client, backend, progress, shutdown).await,
        }
    }
}

fn adopt(stream: std::net::TcpStream) -> io::Result<TcpStream> {
    stream.set_nonblocking(true)?;
    TcpStream::from_std(stream)
}

#[derive(Debug, Default)]
struct Progress {
    client_to_backend: FrozenDirection,
    backend_to_client: FrozenDirection,
    first_closed: Option<RelaySide>,
}

impl Progress {
    fn check(&self) -> io::Result<()> {
        let closed_with_tail =
            |direction: &FrozenDirection| direction.source_eof && !direction.tail.is_empty();
        if closed_with_tail(&self.client_to_backend) || closed_with_tail(&self.backend_to_client) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a direction that reached end of stream cannot carry unwritten bytes",
            ));
        }
        Ok(())
    }

    fn failed(&self, e: io::Error) -> ForwardResult {
        ForwardResult {
            client_to_backend: self.client_to_backend.copied,
            backend_to_client: self.backend_to_client.copied,
            reason: ForwardEndReason::Error(e),
        }
    }
}

enum PumpEnd {
    Eof,
    Failed(io::Error),
    Frozen(Bytes),
}

impl PumpEnd {
    const fn is_frozen(&self) -> bool {
        matches!(self, Self::Frozen(_))
    }

    const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    fn counted(self, copied: u64) -> io::Result<u64> {
        match self {
            Self::Failed(e) => Err(e),
            Self::Eof | Self::Frozen(_) => Ok(copied),
        }
    }

    fn settled(self, copied: u64) -> io::Result<FrozenDirection> {
        match self {
            Self::Eof => Ok(FrozenDirection {
                tail: Bytes::new(),
                source_eof: true,
                copied,
            }),
            Self::Frozen(tail) => Ok(FrozenDirection {
                tail,
                source_eof: false,
                copied,
            }),
            Self::Failed(e) => Err(e),
        }
    }
}

enum Delivery {
    Done,
    Frozen(usize),
}

async fn deliver<F>(
    len: usize,
    mut write: impl FnMut(&mut Context<'_>, usize) -> Poll<io::Result<usize>>,
    frozen: Pin<&mut F>,
    deadline: Duration,
) -> io::Result<Delivery>
where
    F: Future<Output = ()>,
{
    let mut written = 0;
    if write_until(len, &mut written, &mut write, frozen).await? {
        return Ok(Delivery::Done);
    }
    let expiry = tokio::time::sleep(deadline);
    tokio::pin!(expiry);
    write_until(len, &mut written, &mut write, expiry).await?;
    Ok(Delivery::Frozen(written))
}

async fn write_until<S>(
    len: usize,
    written: &mut usize,
    write: &mut impl FnMut(&mut Context<'_>, usize) -> Poll<io::Result<usize>>,
    mut stop: Pin<&mut S>,
) -> io::Result<bool>
where
    S: Future<Output = ()>,
{
    while *written < len {
        let at = *written;
        let result = tokio::select! {
            biased;
            () = stop.as_mut() => return Ok(false),
            result = std::future::poll_fn(|cx| write(cx, at)) => result,
        };
        match result {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "the destination accepted no bytes",
                ));
            }
            Ok(n) => *written += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

fn frozen(
    streams: io::Result<(std::net::TcpStream, std::net::TcpStream)>,
    client_to_backend: io::Result<FrozenDirection>,
    backend_to_client: io::Result<FrozenDirection>,
    first_closed: Option<RelaySide>,
) -> ForwardResult {
    let copied = |direction: &io::Result<FrozenDirection>| {
        direction.as_ref().map_or(0, |direction| direction.copied)
    };
    let counts = (copied(&client_to_backend), copied(&backend_to_client));
    let reason = match frozen_relay(streams, client_to_backend, backend_to_client, first_closed) {
        Ok(relay) => ForwardEndReason::Frozen(Box::new(relay)),
        Err(e) => ForwardEndReason::Error(e),
    };
    ForwardResult {
        client_to_backend: counts.0,
        backend_to_client: counts.1,
        reason,
    }
}

fn frozen_relay(
    streams: io::Result<(std::net::TcpStream, std::net::TcpStream)>,
    client_to_backend: io::Result<FrozenDirection>,
    backend_to_client: io::Result<FrozenDirection>,
    first_closed: Option<RelaySide>,
) -> io::Result<FrozenRelay> {
    let client_to_backend = client_to_backend?;
    let backend_to_client = backend_to_client?;
    let (client, backend) = streams?;
    Ok(FrozenRelay {
        client,
        backend,
        client_to_backend,
        backend_to_client,
        first_closed,
    })
}

struct Pumped {
    from: OwnedReadHalf,
    to: OwnedWriteHalf,
    copied: u64,
    end: PumpEnd,
}

async fn pump(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    pending: Bytes,
    freeze: Freeze,
) -> Pumped {
    let token = freeze.token().child_token();
    let frozen = token.cancelled();
    tokio::pin!(frozen);
    let deadline = freeze.write_deadline();
    let mut copied = 0;

    let end = 'pump: {
        let delivery = deliver(
            pending.len(),
            |cx, at| poll_write(&mut to, cx, &pending[at..]),
            frozen.as_mut(),
            deadline,
        )
        .await;
        match delivery {
            Ok(Delivery::Done) => copied += pending.len() as u64,
            Ok(Delivery::Frozen(at)) => {
                copied += at as u64;
                break 'pump PumpEnd::Frozen(pending.slice(at..));
            }
            Err(e) => break 'pump PumpEnd::Failed(e),
        }

        let mut buf = vec![0u8; COPY_BUFFER_SIZE].into_boxed_slice();
        loop {
            let read = tokio::select! {
                biased;
                () = frozen.as_mut() => break 'pump PumpEnd::Frozen(Bytes::new()),
                read = from.read(&mut buf) => read,
            };
            let filled = match read {
                Ok(0) => break 'pump PumpEnd::Eof,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => break 'pump PumpEnd::Failed(e),
            };
            let chunk = &buf[..filled];
            let delivery = deliver(
                filled,
                |cx, at| poll_write(&mut to, cx, &chunk[at..]),
                frozen.as_mut(),
                deadline,
            )
            .await;
            match delivery {
                Ok(Delivery::Done) => copied += filled as u64,
                Ok(Delivery::Frozen(at)) => {
                    copied += at as u64;
                    break 'pump PumpEnd::Frozen(Bytes::copy_from_slice(&chunk[at..]));
                }
                Err(e) => break 'pump PumpEnd::Failed(e),
            }
        }
    };

    Pumped {
        from,
        to,
        copied,
        end,
    }
}

fn poll_write(
    to: &mut OwnedWriteHalf,
    cx: &mut Context<'_>,
    bytes: &[u8],
) -> Poll<io::Result<usize>> {
    Pin::new(to).poll_write(cx, bytes)
}

async fn copy_until_eof(
    from: OwnedReadHalf,
    to: OwnedWriteHalf,
    pending: Bytes,
    first: Arc<OnceLock<RelaySide>>,
    side: RelaySide,
    freeze: Freeze,
) -> Pumped {
    let mut pumped = pump(from, to, pending, freeze).await;
    if !pumped.end.is_frozen() {
        let _ = first.set(side);
        let _ = pumped.to.shutdown().await;
    }
    pumped
}

enum Leg {
    Running(JoinHandle<Pumped>),
    Ended(io::Result<Pumped>),
}

impl Leg {
    fn start(
        from: OwnedReadHalf,
        to: OwnedWriteHalf,
        progress: FrozenDirection,
        first: &Arc<OnceLock<RelaySide>>,
        side: RelaySide,
        freeze: &Freeze,
    ) -> Self {
        if progress.source_eof {
            return Self::Ended(Ok(Pumped {
                from,
                to,
                copied: 0,
                end: PumpEnd::Eof,
            }));
        }
        Self::Running(tokio::spawn(copy_until_eof(
            from,
            to,
            progress.tail,
            Arc::clone(first),
            side,
            freeze.clone(),
        )))
    }

    const fn running(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    const fn failed(&self) -> bool {
        match self {
            Self::Running(_) => false,
            Self::Ended(Ok(pumped)) => pumped.end.is_failed(),
            Self::Ended(Err(_)) => true,
        }
    }

    async fn join(&mut self) -> io::Result<Pumped> {
        match self {
            Self::Running(task) => joined(task.await),
            Self::Ended(_) => std::future::pending().await,
        }
    }

    fn abort(&self) {
        if let Self::Running(task) = self {
            task.abort();
        }
    }

    async fn end(self) -> Option<io::Result<Pumped>> {
        match self {
            Self::Running(task) => task.await.ok().map(Ok),
            Self::Ended(ended) => Some(ended),
        }
    }
}

fn joined(result: Result<Pumped, JoinError>) -> io::Result<Pumped> {
    result.map_err(io::Error::other)
}

async fn copy_forward(
    client: TcpStream,
    backend: TcpStream,
    progress: Progress,
    shutdown: CancellationToken,
    freeze: Freeze,
) -> ForwardResult {
    let Progress {
        client_to_backend,
        backend_to_client,
        first_closed,
    } = progress;
    let bases = (client_to_backend.copied, backend_to_client.copied);
    let (client_read, client_write) = client.into_split();
    let (backend_read, backend_write) = backend.into_split();
    let first = Arc::new(OnceLock::new());
    if let Some(side) = first_closed {
        let _ = first.set(side);
    }

    let mut c2b = Leg::start(
        client_read,
        backend_write,
        client_to_backend,
        &first,
        RelaySide::Client,
        &freeze,
    );
    let mut b2c = Leg::start(
        backend_read,
        client_write,
        backend_to_client,
        &first,
        RelaySide::Backend,
        &freeze,
    );

    let mut stopped = false;
    while c2b.running() || b2c.running() {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                stopped = true;
                break;
            }
            ended = c2b.join(), if c2b.running() => c2b = Leg::Ended(ended),
            ended = b2c.join(), if b2c.running() => b2c = Leg::Ended(ended),
        }
        if c2b.failed() || b2c.failed() {
            break;
        }
    }

    c2b.abort();
    b2c.abort();
    let c2b = c2b.end().await;
    let b2c = b2c.end().await;
    let first = first.get().copied();
    match (c2b, b2c) {
        (Some(Ok(c2b)), Some(Ok(b2c)))
            if !stopped && (c2b.end.is_frozen() || b2c.end.is_frozen()) =>
        {
            freeze_copy(c2b, b2c, bases, first)
        }
        (c2b, b2c) => finish(first, settled(c2b, bases.0), settled(b2c, bases.1)),
    }
}

fn settled(leg: Option<io::Result<Pumped>>, base: u64) -> Option<io::Result<u64>> {
    leg.map(|ended| ended.and_then(|pumped| pumped.end.counted(base + pumped.copied)))
}

fn freeze_copy(
    c2b: Pumped,
    b2c: Pumped,
    bases: (u64, u64),
    first_closed: Option<RelaySide>,
) -> ForwardResult {
    let client_to_backend = c2b.end.settled(bases.0 + c2b.copied);
    let backend_to_client = b2c.end.settled(bases.1 + b2c.copied);
    let streams =
        reunite(c2b.from, b2c.to).and_then(|client| Ok((client, reunite(b2c.from, c2b.to)?)));
    frozen(streams, client_to_backend, backend_to_client, first_closed)
}

fn reunite(read: OwnedReadHalf, write: OwnedWriteHalf) -> io::Result<std::net::TcpStream> {
    read.reunite(write).map_err(io::Error::other)?.into_std()
}

fn finish(
    first: Option<RelaySide>,
    c2b: Option<io::Result<u64>>,
    b2c: Option<io::Result<u64>>,
) -> ForwardResult {
    let client_to_backend = transferred(c2b.as_ref());
    let backend_to_client = transferred(b2c.as_ref());
    let reason = match first {
        Some(RelaySide::Client) => blame(c2b, ForwardEndReason::ClientClosed),
        Some(RelaySide::Backend) => blame(b2c, ForwardEndReason::BackendClosed),
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
    use super::{CancellationToken, ForwardResult, Progress, RelaySide, TcpStream, finish};
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

    pub(super) async fn splice_forward(
        mut client: TcpStream,
        mut backend: TcpStream,
        progress: Progress,
        shutdown: CancellationToken,
    ) -> ForwardResult {
        use tokio::io::AsyncWriteExt;

        for (to, tail) in [
            (&mut backend, &progress.client_to_backend.tail),
            (&mut client, &progress.backend_to_client.tail),
        ] {
            if let Err(e) = to.write_all(tail).await {
                return progress.failed(e);
            }
        }
        let pipe_c2b = match KernelPipe::new(PIPE_SIZE) {
            Ok(pipe) => pipe,
            Err(e) => return progress.failed(e),
        };
        let pipe_b2c = match KernelPipe::new(PIPE_SIZE) {
            Ok(pipe) => pipe,
            Err(e) => return progress.failed(e),
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
                ended = &mut c2b, if c2b_end.is_none() => (RelaySide::Client, ended),
                ended = &mut b2c, if b2c_end.is_none() => (RelaySide::Backend, ended),
            };
            let Some(ended) = ended.transpose() else {
                break;
            };
            let failed = ended.is_err();
            first.get_or_insert(side);
            match side {
                RelaySide::Client => c2b_end = Some(ended),
                RelaySide::Backend => b2c_end = Some(ended),
            }
            if failed {
                break;
            }
        }
        finish(first, c2b_end, b2c_end)
    }
}
