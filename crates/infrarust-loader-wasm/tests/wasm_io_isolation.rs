#![cfg(all(feature = "wasm", wasm_fixtures_available))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fault_lab;
mod support;

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::build_codec_chains;
use tokio::io::AsyncReadExt;

use fault_lab::faults::{self, Mode};
use fault_lab::{LAB, Lab, LabOptions, LabPlugin};

const VICTIMS: usize = 16;
const FEED_EVERY: Duration = Duration::from_millis(2);
const SPIN_BUDGET: &str = "600ms";

struct Wire {
    origin: Instant,
    listener: std::net::TcpListener,
    writers: Vec<std::net::TcpStream>,
}

impl Wire {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            listener: std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            writers: Vec::new(),
        }
    }

    fn connect(&mut self) -> Inbox {
        let writer = std::net::TcpStream::connect(self.listener.local_addr().unwrap()).unwrap();
        writer.set_nodelay(true).unwrap();
        let (reader, _) = self.listener.accept().unwrap();
        reader.set_nonblocking(true).unwrap();
        self.writers.push(writer);
        Inbox {
            origin: self.origin,
            reader: tokio::net::TcpStream::from_std(reader).unwrap(),
        }
    }

    fn feed(self, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let Self {
                origin,
                listener: _listener,
                mut writers,
            } = self;
            let gap = FEED_EVERY / u32::try_from(writers.len().max(1)).unwrap();
            let mut next = 0;
            while !stop.load(Ordering::Relaxed) {
                let at = u64::try_from(origin.elapsed().as_nanos()).unwrap();
                let _ = writers[next].write_all(&at.to_le_bytes());
                next = (next + 1) % writers.len();
                std::thread::sleep(gap);
            }
        })
    }
}

struct Inbox {
    origin: Instant,
    reader: tokio::net::TcpStream,
}

impl Inbox {
    async fn recv(&mut self) -> Option<Instant> {
        let mut stamp = [0u8; 8];
        self.reader.read_exact(&mut stamp).await.ok()?;
        Some(self.origin + Duration::from_nanos(u64::from_le_bytes(stamp)))
    }
}

struct Victims {
    stop: Arc<AtomicBool>,
    feeder: std::thread::JoinHandle<()>,
    readers: Vec<tokio::task::JoinHandle<Vec<(Instant, Duration)>>>,
    trigger: std::net::TcpStream,
}

impl Victims {
    fn start(on_trigger: impl FnOnce() + Send + 'static) -> Self {
        let mut wire = Wire::new();
        let mut readers = Vec::with_capacity(VICTIMS);
        for _ in 0..VICTIMS {
            let mut inbox = wire.connect();
            readers.push(tokio::spawn(async move {
                let mut samples = Vec::new();
                while let Some(sent) = inbox.recv().await {
                    samples.push((sent, sent.elapsed()));
                }
                samples
            }));
        }
        let mut trigger_wire = Wire::new();
        let mut trigger_inbox = trigger_wire.connect();
        let trigger = trigger_wire.writers.pop().unwrap();
        tokio::spawn(async move {
            if trigger_inbox.recv().await.is_some() {
                on_trigger();
            }
        });
        let stop = Arc::new(AtomicBool::new(false));
        let feeder = wire.feed(Arc::clone(&stop));
        Self {
            stop,
            feeder,
            readers,
            trigger,
        }
    }

    fn fire(&mut self) -> Instant {
        std::thread::sleep(Duration::from_millis(300));
        let at = Instant::now();
        self.trigger.write_all(&0u64.to_le_bytes()).unwrap();
        at
    }

    async fn worst_since(self, since: Instant, span: Duration) -> Duration {
        std::thread::sleep(span + Duration::from_millis(200));
        self.stop.store(true, Ordering::Relaxed);
        self.feeder.join().unwrap();
        let mut worst = Duration::ZERO;
        for reader in self.readers {
            for (sent, latency) in reader.await.unwrap() {
                if sent >= since && sent < since + span {
                    worst = worst.max(latency);
                }
            }
        }
        println!("worst socket wait over {span:?}: {worst:?}");
        worst
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_spinning_event_listener_does_not_stop_a_one_worker_proxy_from_reading_its_sockets() {
    spinning_listener_against_sockets().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spinning_event_listener_does_not_stop_the_proxy_from_reading_its_sockets() {
    spinning_listener_against_sockets().await;
}

async fn spinning_listener_against_sockets() {
    let lab = Arc::new(
        Lab::start(
            vec![LabPlugin::lab("event:chat-message spin")],
            LabOptions {
                proxy_toml: format!(
                    "[wasm]\ncpu_budget = \"{SPIN_BUDGET}\"\n\n[wasm.recovery]\nmax_restarts = 1000\n"
                ),
                ..LabOptions::default()
            },
        )
        .await,
    );
    let bus = Arc::clone(&lab);
    let mut victims = Victims::start(move || {
        tokio::spawn(async move {
            let _ = bus.event_bus.fire(fault_lab::chat()).await;
        });
    });
    let fired = victims.fire();
    let worst = victims.worst_since(fired, Duration::from_millis(600)).await;
    assert!(
        worst < Duration::from_millis(60),
        "sockets waited {worst:?} while a listener spun on another task"
    );
    lab.wait_for("the spinning listener to be cut at its budget", || {
        fault_lab::count_prefix(&lab.log(LAB), "enable recovered") >= 1
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spinning_codec_filter_does_not_stop_the_proxy_from_reading_its_sockets() {
    let lab = Arc::new(
        Lab::start(
            vec![LabPlugin::lab("").grant("codec-filter")],
            LabOptions::default(),
        )
        .await,
    );
    let codecs = Arc::clone(&lab.codecs);
    let mut victims = Victims::start(move || {
        let (mut client, mut server) = build_codec_chains(
            &codecs,
            ProtocolVersion::new(767),
            7,
            "127.0.0.2:1".parse().unwrap(),
            None,
        );
        let mut packet = RawPacket::new(
            faults::CODEC_FAULT_PACKET_BASE + i32::try_from(Mode::Spin.code()).unwrap(),
            Bytes::from_static(b"x"),
        );
        let _ = client.process(&mut packet);
        client.close();
        server.close();
    });
    let fired = victims.fire();
    let worst = victims.worst_since(fired, Duration::from_millis(800)).await;
    assert!(
        worst < Duration::from_millis(60),
        "sockets waited {worst:?} while a codec filter spun on another task"
    );
}

#[test]
fn a_runtime_dropped_while_a_listener_spins_shuts_down() {
    for round in 0..4 {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let lab = Arc::new(
                    Lab::start(
                        vec![LabPlugin::lab("event:chat-message spin")],
                        LabOptions {
                            proxy_toml: "[wasm]\ncpu_budget = \"5s\"\n".to_owned(),
                            ..LabOptions::default()
                        },
                    )
                    .await,
                );
                let bus = Arc::clone(&lab);
                tokio::spawn(async move {
                    let _ = bus.event_bus.fire(fault_lab::chat()).await;
                });
                tokio::time::sleep(Duration::from_millis(200)).await;
            });
            drop(runtime);
            let _ = done.send(());
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(30)).is_ok(),
            "round {round}: the runtime did not finish dropping while a listener spun"
        );
    }
}
