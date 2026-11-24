#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpSocket, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_transport::forward::{
    ForwardEndReason, ForwardResult, Forwarder, Freeze, FrozenDirection, FrozenRelay, RelaySide,
};

const T: Duration = Duration::from_secs(10);
const SOCKET_BUFFER: u32 = 32 * 1024;
const STREAM: usize = 2 * 1024 * 1024;
const QUIET: Duration = Duration::from_millis(150);

macro_rules! forwarder_cases {
    ($($case:ident),+ $(,)?) => {
        $(
            mod $case {
                #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
                async fn copy() {
                    super::$case(super::Forwarder::Copy).await;
                }
            }
        )+
    };
}

forwarder_cases!(
    freeze_then_resume_delivers_every_byte_once,
    idle_freeze_returns_at_once_with_empty_tails,
    half_closed_relay_freezes_and_resumes,
    freeze_stops_writing_at_the_deadline,
    never_freezing_relays_everything,
    refreeze_during_the_tail_write_keeps_the_rest,
    resume_rejects_a_finished_direction_with_a_tail,
);

fn small_socket() -> TcpSocket {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(SOCKET_BUFFER).unwrap();
    socket.set_send_buffer_size(SOCKET_BUFFER).unwrap();
    socket
}

async fn connected_pair() -> (TcpStream, TcpStream) {
    let listening = small_socket();
    listening.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = listening.listen(1).unwrap();
    let connecting = small_socket().connect(listener.local_addr().unwrap());
    let (connected, accepted) = tokio::join!(connecting, listener.accept());
    (connected.unwrap(), accepted.unwrap().0)
}

fn numbered(seed: u32, len: usize) -> Arc<Vec<u8>> {
    let mut stream: Vec<u8> = (0..len.div_ceil(4))
        .flat_map(|i| seed.wrapping_add(u32::try_from(i).unwrap()).to_le_bytes())
        .collect();
    stream.truncate(len);
    Arc::new(stream)
}

struct Rig {
    client: TcpStream,
    backend: TcpStream,
    relay: JoinHandle<ForwardResult>,
    freeze: CancellationToken,
}

async fn start(forwarder: Forwarder, write_deadline: Duration) -> Rig {
    let (client, proxy_client) = connected_pair().await;
    let (proxy_backend, backend) = connected_pair().await;
    let freeze = CancellationToken::new();
    let relay = tokio::spawn(forwarder.forward(
        proxy_client,
        proxy_backend,
        CancellationToken::new(),
        Freeze::new(freeze.clone(), write_deadline),
    ));
    Rig {
        client,
        backend,
        relay,
        freeze,
    }
}

async fn ended(relay: JoinHandle<ForwardResult>) -> ForwardResult {
    tokio::time::timeout(T, relay)
        .await
        .expect("the relay never ended")
        .unwrap()
}

async fn frozen(relay: JoinHandle<ForwardResult>) -> (ForwardResult, FrozenRelay) {
    let mut result = ended(relay).await;
    let reason = std::mem::replace(&mut result.reason, ForwardEndReason::Shutdown);
    let ForwardEndReason::Frozen(relay) = reason else {
        panic!("expected a frozen relay, got {reason:?}");
    };
    (result, *relay)
}

struct Sender {
    task: JoinHandle<OwnedWriteHalf>,
    sent: Arc<AtomicUsize>,
}

impl Sender {
    fn spawn(mut half: OwnedWriteHalf, stream: Arc<Vec<u8>>) -> Self {
        let sent = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let sent = Arc::clone(&sent);
            async move {
                for chunk in stream.chunks(4096) {
                    half.write_all(chunk).await.unwrap();
                    sent.fetch_add(chunk.len(), Ordering::AcqRel);
                }
                half
            }
        });
        Self { task, sent }
    }

    async fn stalled(&self) {
        tokio::time::timeout(T, async {
            let mut last = usize::MAX;
            let mut quiet = 0;
            while quiet < 2 {
                let now = self.sent.load(Ordering::Acquire);
                quiet = if now == last { quiet + 1 } else { 0 };
                last = now;
                tokio::time::sleep(QUIET).await;
            }
        })
        .await
        .expect("the sender never stalled");
        assert!(
            self.sent.load(Ordering::Acquire) < STREAM,
            "the whole stream fit in socket buffers"
        );
    }

    async fn finished(self) -> OwnedWriteHalf {
        tokio::time::timeout(T, self.task)
            .await
            .expect("the sender never finished")
            .unwrap()
    }
}

async fn read_exactly(half: &mut OwnedReadHalf, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    tokio::time::timeout(T, half.read_exact(&mut buf))
        .await
        .expect("the expected bytes never arrived")
        .unwrap();
    buf
}

async fn nothing_more(half: &mut OwnedReadHalf) {
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(QUIET, half.read(&mut byte)).await;
    assert!(read.is_err(), "unexpected read after the freeze: {read:?}");
}

async fn eof(half: &mut OwnedReadHalf) {
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(T, half.read(&mut byte))
        .await
        .expect("the proxy never closed this side")
        .unwrap();
    assert_eq!(read, 0);
}

async fn check_split(
    direction: &FrozenDirection,
    stream: &[u8],
    receiver: &mut OwnedReadHalf,
) -> usize {
    let copied = usize::try_from(direction.copied).unwrap();
    let delivered = read_exactly(receiver, copied).await;
    assert!(delivered == stream[..copied], "delivered bytes diverge");
    nothing_more(receiver).await;
    let tail_end = copied + direction.tail.len();
    assert!(
        direction.tail[..] == stream[copied..tail_end],
        "the tail is not the next slice of the stream"
    );
    copied
}

async fn freeze_then_resume_delivers_every_byte_once(forwarder: Forwarder) {
    let rig = start(forwarder, Duration::from_millis(50)).await;
    let (mut client_read, client_write) = rig.client.into_split();
    let (mut backend_read, backend_write) = rig.backend.into_split();
    let upstream = numbered(0x1000_0000, STREAM);
    let downstream = numbered(0x2000_0000, STREAM);
    let up = Sender::spawn(client_write, Arc::clone(&upstream));
    let down = Sender::spawn(backend_write, Arc::clone(&downstream));
    up.stalled().await;
    down.stalled().await;

    rig.freeze.cancel();
    let (result, relay) = frozen(rig.relay).await;

    assert_eq!(result.client_to_backend, relay.client_to_backend.copied);
    assert_eq!(result.backend_to_client, relay.backend_to_client.copied);
    assert_eq!(relay.first_closed, None);
    for direction in [&relay.client_to_backend, &relay.backend_to_client] {
        assert!(!direction.source_eof);
        assert!(!direction.tail.is_empty(), "a blocked writer left no tail");
    }
    let up_copied = check_split(&relay.client_to_backend, &upstream, &mut backend_read).await;
    let down_copied = check_split(&relay.backend_to_client, &downstream, &mut client_read).await;

    let resumed = tokio::spawn(forwarder.resume(relay, CancellationToken::new(), Freeze::never()));
    let (up_rest, down_rest) = tokio::join!(
        read_exactly(&mut backend_read, STREAM - up_copied),
        read_exactly(&mut client_read, STREAM - down_copied),
    );
    assert!(
        up_rest == upstream[up_copied..],
        "upstream diverged after resume"
    );
    assert!(
        down_rest == downstream[down_copied..],
        "downstream diverged after resume"
    );

    let mut client_write = up.finished().await;
    let mut backend_write = down.finished().await;
    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    backend_write.shutdown().await.unwrap();
    eof(&mut client_read).await;
    let result = ended(resumed).await;

    assert!(
        matches!(result.reason, ForwardEndReason::ClientClosed),
        "{:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, STREAM as u64);
    assert_eq!(result.backend_to_client, STREAM as u64);
}

async fn idle_freeze_returns_at_once_with_empty_tails(forwarder: Forwarder) {
    let rig = start(forwarder, Duration::from_secs(5)).await;
    let (mut client_read, mut client_write) = rig.client.into_split();
    let (mut backend_read, mut backend_write) = rig.backend.into_split();
    client_write.write_all(b"hi").await.unwrap();
    assert_eq!(read_exactly(&mut backend_read, 2).await, b"hi");
    backend_write.write_all(b"hey").await.unwrap();
    assert_eq!(read_exactly(&mut client_read, 3).await, b"hey");

    let started = Instant::now();
    rig.freeze.cancel();
    let (result, relay) = frozen(rig.relay).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "an idle freeze waited {elapsed:?}"
    );
    assert_eq!(result.client_to_backend, 2);
    assert_eq!(result.backend_to_client, 3);
    for (direction, copied) in [(&relay.client_to_backend, 2), (&relay.backend_to_client, 3)] {
        assert!(direction.tail.is_empty());
        assert!(!direction.source_eof);
        assert_eq!(direction.copied, copied);
    }
    assert_eq!(relay.first_closed, None);

    let resumed = tokio::spawn(forwarder.resume(relay, CancellationToken::new(), Freeze::never()));
    client_write.write_all(b"again").await.unwrap();
    assert_eq!(read_exactly(&mut backend_read, 5).await, b"again");
    backend_write.write_all(b"back").await.unwrap();
    assert_eq!(read_exactly(&mut client_read, 4).await, b"back");
    backend_write.shutdown().await.unwrap();
    eof(&mut client_read).await;
    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    let result = ended(resumed).await;

    assert!(
        matches!(result.reason, ForwardEndReason::BackendClosed),
        "{:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, 7);
    assert_eq!(result.backend_to_client, 7);
}

async fn half_closed_relay_freezes_and_resumes(forwarder: Forwarder) {
    let rig = start(forwarder, Duration::from_millis(100)).await;
    let (mut client_read, mut client_write) = rig.client.into_split();
    let (mut backend_read, mut backend_write) = rig.backend.into_split();
    client_write.write_all(b"request").await.unwrap();
    client_write.shutdown().await.unwrap();
    let mut received = Vec::new();
    tokio::time::timeout(T, backend_read.read_to_end(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        received, b"request",
        "the backend should see the request then EOF"
    );

    rig.freeze.cancel();
    let (result, relay) = frozen(rig.relay).await;

    assert_eq!(result.client_to_backend, 7);
    assert!(relay.client_to_backend.source_eof);
    assert!(relay.client_to_backend.tail.is_empty());
    assert_eq!(relay.client_to_backend.copied, 7);
    assert!(!relay.backend_to_client.source_eof);
    assert!(relay.backend_to_client.tail.is_empty());
    assert_eq!(relay.backend_to_client.copied, 0);
    assert_eq!(relay.first_closed, Some(RelaySide::Client));

    let resumed = tokio::spawn(forwarder.resume(relay, CancellationToken::new(), Freeze::never()));
    backend_write.write_all(b"reply").await.unwrap();
    assert_eq!(read_exactly(&mut client_read, 5).await, b"reply");
    backend_write.write_all(b" more").await.unwrap();
    backend_write.shutdown().await.unwrap();
    let mut returned = Vec::new();
    tokio::time::timeout(T, client_read.read_to_end(&mut returned))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned, b" more");
    let result = ended(resumed).await;

    assert!(
        matches!(result.reason, ForwardEndReason::ClientClosed),
        "{:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, 7);
    assert_eq!(result.backend_to_client, 10);
}

async fn freeze_stops_writing_at_the_deadline(forwarder: Forwarder) {
    let deadline = Duration::from_millis(300);
    let rig = start(forwarder, deadline).await;
    let (_client_read, client_write) = rig.client.into_split();
    let (mut backend_read, _backend_write) = rig.backend.into_split();
    let upstream = numbered(0x3000_0000, STREAM);
    let up = Sender::spawn(client_write, Arc::clone(&upstream));
    up.stalled().await;

    let started = Instant::now();
    rig.freeze.cancel();
    let (result, relay) = frozen(rig.relay).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= deadline,
        "the freeze gave up after {elapsed:?}, before the deadline"
    );
    assert!(
        elapsed < deadline + Duration::from_millis(500),
        "the freeze overran its deadline: {elapsed:?}"
    );
    assert!(!relay.client_to_backend.tail.is_empty());
    assert!(!relay.client_to_backend.source_eof);
    assert_eq!(result.client_to_backend, relay.client_to_backend.copied);
    assert!(relay.backend_to_client.tail.is_empty());
    check_split(&relay.client_to_backend, &upstream, &mut backend_read).await;
    up.task.abort();
}

async fn never_freezing_relays_everything(forwarder: Forwarder) {
    let (client, proxy_client) = connected_pair().await;
    let (proxy_backend, backend) = connected_pair().await;
    let relay = tokio::spawn(forwarder.forward(
        proxy_client,
        proxy_backend,
        CancellationToken::new(),
        Freeze::never(),
    ));
    let (mut client_read, client_write) = client.into_split();
    let (mut backend_read, backend_write) = backend.into_split();
    let upstream = numbered(0x4000_0000, STREAM);
    let downstream = numbered(0x5000_0000, STREAM);
    let up = Sender::spawn(client_write, Arc::clone(&upstream));
    let down = Sender::spawn(backend_write, Arc::clone(&downstream));

    let (up_received, down_received) = tokio::join!(
        read_exactly(&mut backend_read, STREAM),
        read_exactly(&mut client_read, STREAM),
    );
    assert!(up_received == *upstream);
    assert!(down_received == *downstream);

    let mut backend_write = down.finished().await;
    let mut client_write = up.finished().await;
    backend_write.shutdown().await.unwrap();
    eof(&mut client_read).await;
    client_write.shutdown().await.unwrap();
    eof(&mut backend_read).await;
    let result = ended(relay).await;

    assert!(
        matches!(result.reason, ForwardEndReason::BackendClosed),
        "{:?}",
        result.reason
    );
    assert_eq!(result.client_to_backend, STREAM as u64);
    assert_eq!(result.backend_to_client, STREAM as u64);
}

async fn refreeze_during_the_tail_write_keeps_the_rest(forwarder: Forwarder) {
    let (_client, proxy_client) = connected_pair().await;
    let (proxy_backend, backend) = connected_pair().await;
    let tail = numbered(0x6000_0000, STREAM);
    let relay = FrozenRelay {
        client: proxy_client.into_std().unwrap(),
        backend: proxy_backend.into_std().unwrap(),
        client_to_backend: FrozenDirection {
            tail: Bytes::copy_from_slice(&tail),
            source_eof: false,
            copied: 1000,
        },
        backend_to_client: FrozenDirection {
            tail: Bytes::new(),
            source_eof: false,
            copied: 2000,
        },
        first_closed: None,
    };
    let freeze = CancellationToken::new();
    let resumed = tokio::spawn(forwarder.resume(
        relay,
        CancellationToken::new(),
        Freeze::new(freeze.clone(), Duration::from_millis(50)),
    ));
    tokio::time::timeout(T, backend.readable())
        .await
        .expect("the resumed relay never wrote its tail")
        .unwrap();
    tokio::time::sleep(QUIET).await;

    freeze.cancel();
    let (result, relay) = frozen(resumed).await;

    let written = usize::try_from(relay.client_to_backend.copied - 1000).unwrap();
    assert!(written > 0, "the resumed relay wrote none of its tail");
    assert!(written < STREAM, "the whole tail fit in socket buffers");
    assert!(
        relay.client_to_backend.tail[..] == tail[written..],
        "the second tail is not the unwritten end of the first"
    );
    assert!(!relay.client_to_backend.source_eof);
    assert_eq!(relay.backend_to_client.copied, 2000);
    assert!(relay.backend_to_client.tail.is_empty());
    assert_eq!(result.client_to_backend, 1000 + written as u64);
    assert_eq!(result.backend_to_client, 2000);
    let (mut backend_read, _backend_write) = backend.into_split();
    let delivered = read_exactly(&mut backend_read, written).await;
    assert!(delivered == tail[..written]);
    nothing_more(&mut backend_read).await;
}

async fn resume_rejects_a_finished_direction_with_a_tail(forwarder: Forwarder) {
    let (_client, proxy_client) = connected_pair().await;
    let (proxy_backend, _backend) = connected_pair().await;
    let relay = FrozenRelay {
        client: proxy_client.into_std().unwrap(),
        backend: proxy_backend.into_std().unwrap(),
        client_to_backend: FrozenDirection {
            tail: Bytes::from_static(b"late"),
            source_eof: true,
            copied: 4,
        },
        backend_to_client: FrozenDirection::default(),
        first_closed: Some(RelaySide::Client),
    };

    let result = forwarder
        .resume(relay, CancellationToken::new(), Freeze::never())
        .await;

    let ForwardEndReason::Error(e) = result.reason else {
        panic!("expected an error, got {:?}", result.reason);
    };
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(result.client_to_backend, 4);
}
