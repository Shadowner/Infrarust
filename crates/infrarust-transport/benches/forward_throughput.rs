#![allow(clippy::unwrap_used, clippy::expect_used)]
use divan::Bencher;
use divan::counter::BytesCount;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use infrarust_transport::{ForwardResult, Forwarder};

const BURST: usize = 100 * 1024;
const BURSTS: usize = 656;
const BULK_BYTES: usize = BURST * BURSTS;

const PACKET: usize = 64;
const ROUND_TRIPS: usize = 20_000;
const ECHO_BYTES: usize = 2 * PACKET * ROUND_TRIPS;

fn main() {
    divan::main();
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

async fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connecting = TcpStream::connect(listener.local_addr().unwrap());
    let (connected, accepted) = tokio::join!(connecting, listener.accept());
    let (connected, accepted) = (connected.unwrap(), accepted.unwrap().0);
    connected.set_nodelay(true).unwrap();
    accepted.set_nodelay(true).unwrap();
    (connected, accepted)
}

struct Session {
    client: TcpStream,
    backend: TcpStream,
    forwarding: JoinHandle<ForwardResult>,
}

impl Session {
    async fn open(forwarder: Forwarder) -> Self {
        let (client, proxy_client) = connected_pair().await;
        let (proxy_backend, backend) = connected_pair().await;
        let forwarding = tokio::spawn(async move {
            forwarder
                .forward(proxy_client, proxy_backend, CancellationToken::new())
                .await
        });
        Self {
            client,
            backend,
            forwarding,
        }
    }

    async fn bulk(self) {
        let Self {
            mut client,
            mut backend,
            forwarding,
        } = self;
        let sender = tokio::spawn(async move {
            let burst = vec![0xA5u8; BURST];
            for _ in 0..BURSTS {
                client.write_all(&burst).await.unwrap();
            }
            client
        });
        let mut buf = vec![0u8; 64 * 1024];
        let mut received = 0;
        while received < BULK_BYTES {
            let n = backend.read(&mut buf).await.unwrap();
            assert_ne!(n, 0, "forwarder closed after {received} bytes");
            received += n;
        }
        drop((sender.await.unwrap(), backend));
        forwarding.await.unwrap();
    }

    async fn echo(self) {
        let Self {
            mut client,
            mut backend,
            forwarding,
        } = self;
        let echoing = tokio::spawn(async move {
            let mut packet = [0u8; PACKET];
            for _ in 0..ROUND_TRIPS {
                backend.read_exact(&mut packet).await.unwrap();
                backend.write_all(&packet).await.unwrap();
            }
            backend
        });
        let mut packet = [0x5Au8; PACKET];
        for _ in 0..ROUND_TRIPS {
            client.write_all(&packet).await.unwrap();
            client.read_exact(&mut packet).await.unwrap();
        }
        drop((client, echoing.await.unwrap()));
        forwarding.await.unwrap();
    }
}

fn bench_bulk(bencher: Bencher, forwarder: Forwarder) {
    let rt = runtime();
    bencher
        .counter(BytesCount::new(BULK_BYTES))
        .with_inputs(|| rt.block_on(Session::open(forwarder)))
        .bench_local_values(|session| rt.block_on(session.bulk()));
}

fn bench_echo(bencher: Bencher, forwarder: Forwarder) {
    let rt = runtime();
    bencher
        .counter(BytesCount::new(ECHO_BYTES))
        .with_inputs(|| rt.block_on(Session::open(forwarder)))
        .bench_local_values(|session| rt.block_on(session.echo()));
}

#[divan::bench(sample_count = 20, sample_size = 1)]
fn bulk_copy(bencher: Bencher) {
    bench_bulk(bencher, Forwarder::Copy);
}

#[divan::bench(sample_count = 20, sample_size = 1)]
fn small_packets_copy(bencher: Bencher) {
    bench_echo(bencher, Forwarder::Copy);
}

#[cfg(target_os = "linux")]
mod splice {
    use super::{Bencher, Forwarder, bench_bulk, bench_echo};

    #[divan::bench(sample_count = 20, sample_size = 1)]
    fn bulk_splice(bencher: Bencher) {
        bench_bulk(bencher, Forwarder::Splice);
    }

    #[divan::bench(sample_count = 20, sample_size = 1)]
    fn small_packets_splice(bencher: Bencher) {
        bench_echo(bencher, Forwarder::Splice);
    }
}
