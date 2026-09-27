use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use infrarust_api::filter::{
    CodecFilterFactory, CodecFilterInstance, CodecSessionInit, CodecVerdict, FilterMetadata,
    FrameOutput,
};
use infrarust_api::types::{ProtocolVersion, RawPacket};
use infrarust_core::filter::codec_chain::{CodecFilterChain, FilterResult, build_codec_chains};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;

use crate::fault_lab::faults::{self, Mode};
use crate::fault_lab::{Lab, LabOptions, LabPlugin};

const HEALTHY: &str = "scripted";
const MODIFY: &str = "codec-modify";
const PAYLOAD: usize = 512;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn emit(scenario: &str, metric: &str, value: f64) {
    println!("{{\"scenario\":\"{scenario}\",\"metric\":\"{metric}\",\"value\":{value:.3}}}");
}

fn proxy_toml() -> String {
    let mut toml = String::from("[wasm]\n");
    if let Some(tick) = env_string("STUDY_EPOCH_TICK") {
        toml.push_str(&format!("epoch_tick = \"{tick}\"\n"));
    }
    if let Some(budget) = env_string("STUDY_CODEC_BUDGET") {
        toml.push_str(&format!("codec_cpu_budget = \"{budget}\"\n"));
    }
    if let Some(pool) = env_string("STUDY_INSTANCE_POOL") {
        toml.push_str(&format!("instance_pool = {pool}\n"));
    }
    if let Some(faults) = env_string("STUDY_QUARANTINE_FAULTS") {
        toml.push_str(&format!("\n[wasm.codec_quarantine]\nfaults = {faults}\n"));
    }
    toml
}

async fn lab(with_faults: bool) -> Arc<Lab> {
    let plugins = if with_faults {
        vec![LabPlugin::lab("").grant("codec-filter")]
    } else {
        Vec::new()
    };
    let options = LabOptions {
        proxy_toml: proxy_toml(),
        extra: vec![
            (HEALTHY, HEALTHY, "cmd greet record".to_owned()),
            (MODIFY, MODIFY, String::new()),
        ],
        grants: vec![(MODIFY, "codec-filter")],
        ..LabOptions::default()
    };
    let lab = Lab::start(plugins, options).await;
    lab.enable(HEALTHY).await;
    lab.enable(MODIFY).await;
    Arc::new(lab)
}

struct NativePassthroughFactory;

impl CodecFilterFactory for NativePassthroughFactory {
    fn metadata(&self) -> FilterMetadata {
        FilterMetadata::new("native-passthrough")
    }

    fn create(&self, _init: &CodecSessionInit) -> Box<dyn CodecFilterInstance> {
        Box::new(NativePassthrough)
    }
}

struct NativePassthrough;

impl CodecFilterInstance for NativePassthrough {
    fn filter(&mut self, _packet: &mut RawPacket, _output: &mut FrameOutput) -> CodecVerdict {
        CodecVerdict::Pass
    }
}

fn native_registry() -> Arc<CodecFilterRegistryImpl> {
    let registry = CodecFilterRegistryImpl::new();
    registry
        .register_builtin(Box::new(NativePassthroughFactory))
        .unwrap();
    Arc::new(registry)
}

const VICTIM: &str = "127.0.0.1:1";
const ATTACKER: &str = "127.0.0.2:1";

async fn chains(
    registry: &CodecFilterRegistryImpl,
    id: u64,
) -> (CodecFilterChain, CodecFilterChain) {
    chains_from(registry, id, VICTIM)
}

fn chains_from(
    registry: &CodecFilterRegistryImpl,
    id: u64,
    remote: &str,
) -> (CodecFilterChain, CodecFilterChain) {
    build_codec_chains(
        registry,
        ProtocolVersion::new(767),
        id,
        remote.parse().unwrap(),
        None,
    )
}

fn packet(id: i32) -> RawPacket {
    RawPacket::new(id, Bytes::from(vec![0xABu8; PAYLOAD]))
}

fn percentile(samples: &mut [f64], at: f64) -> f64 {
    if samples.is_empty() {
        return f64::NAN;
    }
    samples.sort_by(f64::total_cmp);
    let index = ((samples.len() as f64 - 1.0) * at).round() as usize;
    samples[index]
}

fn max(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::NAN, f64::max)
}

fn cpu_seconds() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .collect();
    let ticks = |at: usize| {
        fields
            .get(at)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (ticks(11) + ticks(12)) / 100.0
}

fn rss_kib() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kib| kib.parse().ok())
        .unwrap_or(0.0)
}

fn threads() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|rest| rest.trim().parse().ok())
        .unwrap_or(0.0)
}

pub(crate) fn main() {
    let scenario = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| "hot".to_owned());
    let workers = env_usize("STUDY_WORKERS", 2);
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if workers > 0 {
        builder.worker_threads(workers);
    }
    eager_handoff(&mut builder);
    let rt = builder.enable_all().build().unwrap();
    rt.block_on(async {
        match scenario.as_str() {
            "hot" => hot().await,
            "rr" => round_robin().await,
            "create" => create().await,
            "load" => load().await,
            "isolation" => isolation().await,
            "idle" => idle().await,
            "sizes" => sizes().await,
            other => panic!("unknown scenario {other}"),
        }
    });
    rt.shutdown_timeout(Duration::from_secs(2));
}

async fn sizes() {
    let lab = lab(false).await;
    let codecs = Arc::clone(&lab.codecs);
    tokio::spawn(async move {
        let (mut client, mut server) = chains(&codecs, 1).await;
        for size in [16 * 1024, 256 * 1024, 2 * 1024 * 1024] {
            for (shape, id) in [("pass", 0x10), ("modified", 0x02)] {
                let template = RawPacket::new(id, Bytes::from(vec![0xABu8; size]));
                let iterations = (64 * 1024 * 1024 / size).clamp(20, 20_000);
                let mut worst = 0f64;
                let started = Instant::now();
                for _ in 0..iterations {
                    let mut p = template.clone();
                    let one = Instant::now();
                    black_box(client.process(&mut p));
                    worst = worst.max(one.elapsed().as_secs_f64() * 1e6);
                }
                let us = started.elapsed().as_secs_f64() * 1e6 / iterations as f64;
                emit("sizes", &format!("{}k.{shape}.mean_us", size / 1024), us);
                emit("sizes", &format!("{}k.{shape}.max_us", size / 1024), worst);
            }
        }
        client.close();
        server.close();
    })
    .await
    .unwrap();
}

async fn idle() {
    let lab = lab(false).await;
    let seconds = env_usize("STUDY_SECONDS", 10);
    std::thread::sleep(Duration::from_millis(500));
    let cpu = cpu_seconds();
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds as u64));
    let cpu = cpu_seconds() - cpu;
    emit(
        "idle",
        "cpu_percent",
        cpu * 100.0 / started.elapsed().as_secs_f64(),
    );
    drop(lab);
}

#[allow(unexpected_cfgs)]
fn eager_handoff(builder: &mut tokio::runtime::Builder) {
    #[cfg(tokio_unstable)]
    if std::env::var_os("STUDY_EAGER_HANDOFF").is_some() {
        builder.enable_eager_driver_handoff();
    }
    let _ = builder;
}

async fn ns_per_packet(
    chain: &mut CodecFilterChain,
    id: i32,
    iterations: usize,
    yielding: bool,
) -> f64 {
    let template = packet(id);
    for _ in 0..iterations / 10 {
        let mut p = template.clone();
        black_box(chain.process(black_box(&mut p)));
        if yielding {
            tokio::task::yield_now().await;
        }
    }
    let started = Instant::now();
    for _ in 0..iterations {
        let mut p = template.clone();
        black_box(chain.process(black_box(&mut p)));
        if yielding {
            tokio::task::yield_now().await;
        }
    }
    started.elapsed().as_nanos() as f64 / iterations as f64
}

async fn hot() {
    let lab = lab(false).await;
    let native = native_registry();
    let iterations = env_usize("STUDY_ITERS", 200_000);
    let codecs = Arc::clone(&lab.codecs);
    tokio::spawn(async move {
        for (label, registry) in [("native", &native), ("wasm", &codecs)] {
            let (mut client, mut server) = chains(registry, 1).await;
            for (shape, id, yielding) in [
                ("pass", 0x10, false),
                ("modified", 0x02, false),
                ("pass_yield", 0x10, true),
            ] {
                let ns = ns_per_packet(&mut client, id, iterations, yielding).await;
                emit("hot", &format!("{label}.{shape}.ns"), ns);
            }
            client.close();
            server.close();
        }
    })
    .await
    .unwrap();
}

async fn round_robin() {
    let lab = lab(false).await;
    let native = native_registry();
    let conns = env_usize("STUDY_CONNS", 1000);
    let iterations = env_usize("STUDY_ITERS", 200_000);
    let codecs = Arc::clone(&lab.codecs);
    tokio::spawn(async move {
        for (label, registry) in [("native", &native), ("wasm", &codecs)] {
            let mut all = Vec::with_capacity(conns);
            for n in 0..conns {
                all.push(chains(registry, n as u64).await);
            }
            let template = packet(0x10);
            for (client, _) in &mut all {
                let mut p = template.clone();
                black_box(client.process(&mut p));
            }
            for (shape, yielding) in [("pass", false), ("pass_yield", true)] {
                let started = Instant::now();
                for n in 0..iterations {
                    let mut p = template.clone();
                    black_box(all[n % conns].0.process(black_box(&mut p)));
                    if yielding {
                        tokio::task::yield_now().await;
                    }
                }
                let ns = started.elapsed().as_nanos() as f64 / iterations as f64;
                emit("rr", &format!("{label}.{shape}.ns"), ns);
            }
            for (client, server) in &mut all {
                client.close();
                server.close();
            }
        }
    })
    .await
    .unwrap();
}

async fn create() {
    let lab = lab(false).await;
    let sequential = env_usize("STUDY_CREATE_SEQ", 2000);
    let concurrent = env_usize("STUDY_CREATE_PAR", 1000);
    let codecs = Arc::clone(&lab.codecs);
    tokio::spawn(async move {
        for n in 0..sequential / 10 {
            let (mut client, mut server) = chains(&codecs, n as u64).await;
            client.close();
            server.close();
        }
        let started = Instant::now();
        for n in 0..sequential {
            let (mut client, mut server) = chains(&codecs, n as u64).await;
            client.close();
            server.close();
        }
        let us = started.elapsed().as_nanos() as f64 / sequential as f64 / 1000.0;
        emit("create", "sequential.us_per_conn", us);

        let rss = rss_kib();
        let cpu = cpu_seconds();
        let started = Instant::now();
        let mut handles = Vec::with_capacity(concurrent);
        for n in 0..concurrent {
            let codecs = Arc::clone(&codecs);
            handles.push(tokio::spawn(async move {
                let begun = Instant::now();
                let pair = chains(&codecs, n as u64).await;
                (pair, begun.elapsed().as_secs_f64() * 1e6)
            }));
        }
        let mut latencies = Vec::with_capacity(concurrent);
        let mut pairs = Vec::with_capacity(concurrent);
        for handle in handles {
            let (pair, us) = handle.await.unwrap();
            latencies.push(us);
            pairs.push(pair);
        }
        let wall = started.elapsed().as_secs_f64();
        let cpu = cpu_seconds() - cpu;
        let mut packets = 0usize;
        for (client, _) in &mut pairs {
            let mut p = packet(0x10);
            black_box(client.process(&mut p));
            packets += 1;
        }
        emit(
            "create",
            "concurrent.rss_kib_per_conn",
            (rss_kib() - rss) / packets.max(1) as f64,
        );
        for (client, server) in &mut pairs {
            client.close();
            server.close();
        }
        emit(
            "create",
            "concurrent.wall_us_per_conn",
            wall * 1e6 / concurrent as f64,
        );
        emit(
            "create",
            "concurrent.cpu_us_per_conn",
            cpu * 1e6 / concurrent as f64,
        );
        emit(
            "create",
            "concurrent.p50_us",
            percentile(&mut latencies, 0.5),
        );
        emit(
            "create",
            "concurrent.p99_us",
            percentile(&mut latencies, 0.99),
        );
        emit("create", "threads", threads());
    })
    .await
    .unwrap();
}

struct Feeder {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<u64>>,
}

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
}

struct Inbox {
    origin: Instant,
    reader: tokio::net::TcpStream,
}

impl Inbox {
    async fn recv(&mut self) -> Option<(Instant, f64)> {
        use tokio::io::AsyncReadExt;
        let mut stamp = [0u8; 8];
        self.reader.read_exact(&mut stamp).await.ok()?;
        let sent = self.origin + Duration::from_nanos(u64::from_le_bytes(stamp));
        Some((sent, 0.0))
    }
}

impl Feeder {
    fn start(wire: Wire, rate_hz: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("study-feeder".to_owned())
            .spawn(move || {
                use std::io::Write;
                let Wire {
                    origin,
                    listener: _listener,
                    mut writers,
                } = wire;
                let total = (writers.len() * rate_hz).max(1);
                let interval = Duration::from_nanos(1_000_000_000 / total as u64);
                let mut due = Instant::now();
                let mut next = 0usize;
                let mut sent = 0u64;
                while !flag.load(Ordering::Relaxed) {
                    due += interval;
                    let now = Instant::now();
                    if due > now {
                        std::thread::sleep(due - now);
                    }
                    let at = u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    let count = writers.len();
                    let _ = writers[next % count].write_all(&at.to_le_bytes());
                    next += 1;
                    sent += 1;
                }
                sent
            })
            .unwrap();
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn stop(&mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().map_or(0, |h| h.join().unwrap())
    }
}

async fn load() {
    let registry: Arc<CodecFilterRegistryImpl> = match env_string("STUDY_LOAD_REGISTRY").as_deref()
    {
        Some("none") => Arc::new(CodecFilterRegistryImpl::new()),
        Some("native") => native_registry(),
        _ => Arc::clone(&lab(false).await.codecs),
    };
    let conns = env_usize("STUDY_CONNS", 1000);
    let rate = env_usize("STUDY_RATE_HZ", 20);
    let seconds = env_usize("STUDY_SECONDS", 5);
    let mut wire = Wire::new();
    let mut tasks = Vec::with_capacity(conns);
    for n in 0..conns {
        let mut inbox = wire.connect();
        let (mut client, mut server) = chains(&registry, n as u64).await;
        tasks.push(tokio::spawn(async move {
            let template = packet(0x10);
            let mut latencies = Vec::new();
            while let Some((sent, _)) = inbox.recv().await {
                let mut p = template.clone();
                black_box(client.process(&mut p));
                latencies.push(sent.elapsed().as_secs_f64() * 1e6);
            }
            client.close();
            server.close();
            latencies
        }));
    }
    std::thread::sleep(Duration::from_millis(200));
    let cpu = cpu_seconds();
    let started = Instant::now();
    let mut feeder = Feeder::start(wire, rate);
    std::thread::sleep(Duration::from_secs(seconds as u64));
    let sent = feeder.stop();
    let wall = started.elapsed().as_secs_f64();
    let mut latencies = Vec::new();
    for task in tasks {
        latencies.extend(task.await.unwrap());
    }
    let cpu = cpu_seconds() - cpu;
    emit("load", "packets", sent as f64);
    emit("load", "p50_us", percentile(&mut latencies, 0.5));
    emit("load", "p99_us", percentile(&mut latencies, 0.99));
    emit("load", "p999_us", percentile(&mut latencies, 0.999));
    emit("load", "cpu_us_per_packet", cpu * 1e6 / sent.max(1) as f64);
    emit("load", "cpu_cores", cpu / wall);
    emit("load", "threads", threads());
}

#[derive(Default)]
struct VictimReport {
    samples: Vec<(Instant, f64)>,
    lost_faultlab: u64,
    lost_modify: u64,
}

async fn victim(
    mut inbox: Inbox,
    mut client: CodecFilterChain,
    mut server: CodecFilterChain,
) -> VictimReport {
    let mut report = VictimReport::default();
    let mut seq = 0u64;
    while let Some((sent, _)) = inbox.recv().await {
        seq += 1;
        match seq % 10 {
            3 => {
                let mut p = packet(faults::CODEC_MARK_PACKET);
                let result = client.process(&mut p);
                if !matches!(result, FilterResult::Pass { .. }) || &p.data[..] != b"fault-lab" {
                    report.lost_faultlab += 1;
                }
            }
            7 => {
                let mut p = packet(0x01);
                if !matches!(client.process(&mut p), FilterResult::Dropped) {
                    report.lost_modify += 1;
                }
            }
            _ => {
                let mut p = packet(0x10);
                black_box(client.process(&mut p));
            }
        }
        report
            .samples
            .push((sent, sent.elapsed().as_secs_f64() * 1e3));
    }
    client.close();
    server.close();
    report
}

struct AttackReport {
    creates: Vec<f64>,
    spins: Vec<f64>,
}

async fn attacker(
    lab: Arc<Lab>,
    stop: Arc<AtomicBool>,
    variant: String,
    period: Duration,
    id: u64,
    mut trigger: Option<Inbox>,
) -> AttackReport {
    let spin_packet = faults::CODEC_FAULT_PACKET_BASE + i32::try_from(Mode::Spin.code()).unwrap();
    let mut report = AttackReport {
        creates: Vec::new(),
        spins: Vec::new(),
    };
    let mut round = 0u64;
    let burst = env_usize("STUDY_BURST_CONNS", 25);
    while !stop.load(Ordering::Relaxed) && variant == "burst" {
        let begun = Instant::now();
        let mut opened = Vec::with_capacity(burst);
        for _ in 0..burst {
            round += 1;
            let created = Instant::now();
            opened.push(chains_from(
                &lab.codecs,
                1_000_000 + id * 100_000 + round,
                ATTACKER,
            ));
            report.creates.push(created.elapsed().as_secs_f64() * 1e3);
            tokio::task::yield_now().await;
        }
        let mut spinning = Vec::with_capacity(burst);
        for (mut client, mut server) in opened {
            spinning.push(tokio::spawn(async move {
                let mut p = RawPacket::new(spin_packet, Bytes::from_static(b"x"));
                let spun = Instant::now();
                black_box(client.process(&mut p));
                let ms = spun.elapsed().as_secs_f64() * 1e3;
                client.close();
                server.close();
                ms
            }));
        }
        for task in spinning {
            report.spins.push(task.await.unwrap());
        }
        tokio::task::yield_now().await;
        let spent = begun.elapsed();
        if spent < period {
            tokio::time::sleep(period - spent).await;
        }
    }
    while !stop.load(Ordering::Relaxed) {
        if let Some(inbox) = trigger.as_mut()
            && inbox.recv().await.is_none()
        {
            break;
        }
        let begun = Instant::now();
        round += 1;
        let connection = if variant == "create" {
            faults::CODEC_FAULT_CONNECTION_BASE + Mode::Spin.code()
        } else {
            1_000_000 + id * 100_000 + round
        };
        let (mut client, mut server) = chains_from(&lab.codecs, connection, ATTACKER);
        let created = begun.elapsed();
        report.creates.push(created.as_secs_f64() * 1e3);
        if variant != "create" {
            let mut p = RawPacket::new(spin_packet, Bytes::from_static(b"x"));
            let spun = Instant::now();
            black_box(client.process(&mut p));
            report.spins.push(spun.elapsed().as_secs_f64() * 1e3);
        }
        client.close();
        server.close();
        if trigger.is_some() {
            continue;
        }
        tokio::task::yield_now().await;
        let spent = begun.elapsed();
        if spent < period {
            tokio::time::sleep(period - spent).await;
        }
    }
    report
}

async fn isolation() {
    let lab = lab(true).await;
    let victims = env_usize("STUDY_VICTIMS", 50);
    let rate = env_usize("STUDY_RATE_HZ", 100);
    let spinners = env_usize("STUDY_SPINNERS", 4);
    let quiet = Duration::from_millis(env_usize("STUDY_QUIET_MS", 1500) as u64);
    let attack = Duration::from_millis(env_usize("STUDY_ATTACK_MS", 4000) as u64);
    let period = Duration::from_millis(env_usize("STUDY_ATTACK_PERIOD_MS", 100) as u64);
    let variant = env_string("STUDY_ATTACK").unwrap_or_else(|| "filter".to_owned());

    let mut wire = Wire::new();
    let mut victim_tasks = Vec::with_capacity(victims);
    for n in 0..victims {
        let inbox = wire.connect();
        let (client, server) = chains(&lab.codecs, 10 + n as u64).await;
        victim_tasks.push(tokio::spawn(victim(inbox, client, server)));
    }

    let probes: Arc<std::sync::Mutex<Vec<(Instant, f64)>>> = Arc::default();
    let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut probe_wire = Wire::new();
    let mut probe_inbox = probe_wire.connect();
    let prober = {
        let lab = Arc::clone(&lab);
        let probes = Arc::clone(&probes);
        let pending = Arc::clone(&pending);
        tokio::spawn(async move {
            while let Some((due, _)) = probe_inbox.recv().await {
                let lab = Arc::clone(&lab);
                let probes = Arc::clone(&probes);
                let pending = Arc::clone(&pending);
                pending.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    lab.dispatch("greet").await;
                    let ms = due.elapsed().as_secs_f64() * 1e3;
                    probes.lock().unwrap().push((due, ms));
                    pending.fetch_sub(1, Ordering::SeqCst);
                });
            }
        })
    };

    std::thread::sleep(Duration::from_millis(300));
    let mut feeder = Feeder::start(wire, rate);
    let mut probe_feeder = Feeder::start(probe_wire, 50);
    let quiet_start = Instant::now();
    std::thread::sleep(quiet);

    let stop_attack = Arc::new(AtomicBool::new(false));
    let attack_start = Instant::now();
    let cpu = cpu_seconds();
    let io_trigger = env_string("STUDY_ATTACK_TRIGGER").as_deref() == Some("io");
    let mut attack_wire = Wire::new();
    let mut attackers = Vec::with_capacity(spinners);
    for n in 0..spinners {
        let trigger = io_trigger.then(|| attack_wire.connect());
        attackers.push(tokio::spawn(attacker(
            Arc::clone(&lab),
            Arc::clone(&stop_attack),
            variant.clone(),
            period,
            n as u64,
            trigger,
        )));
    }
    let mut attack_feeder = io_trigger.then(|| {
        let rate = (1000 / period.as_millis().max(1)) as usize;
        Feeder::start(attack_wire, rate.max(1))
    });
    std::thread::sleep(attack);
    let attack_end = Instant::now();
    if let Some(feeder) = attack_feeder.as_mut() {
        feeder.stop();
    }
    let cpu = cpu_seconds() - cpu;
    stop_attack.store(true, Ordering::Relaxed);
    let peak_threads = threads();
    let mut creates = Vec::new();
    let mut spins = Vec::new();
    let mut attempts = 0usize;
    for handle in attackers {
        let report = handle.await.unwrap();
        attempts += report.creates.len();
        creates.extend(report.creates);
        spins.extend(report.spins);
    }
    feeder.stop();
    probe_feeder.stop();
    prober.await.unwrap();
    let waiting = Instant::now();
    while pending.load(Ordering::SeqCst) > 0 && waiting.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let commands = std::mem::take(&mut *probes.lock().unwrap());
    let mut lost_faultlab = 0;
    let mut lost_modify = 0;
    let mut victim_quiet = Vec::new();
    let mut victim_attack = Vec::new();
    for task in victim_tasks {
        let report = task.await.unwrap();
        lost_faultlab += report.lost_faultlab;
        lost_modify += report.lost_modify;
        for (sent, ms) in report.samples {
            if sent >= quiet_start && sent < attack_start {
                victim_quiet.push(ms);
            } else if sent >= attack_start && sent < attack_end {
                victim_attack.push(ms);
            }
        }
    }
    let mut cmd_quiet = Vec::new();
    let mut cmd_attack = Vec::new();
    for (begun, ms) in commands {
        if begun < attack_start {
            cmd_quiet.push(ms);
        } else if begun < attack_end {
            cmd_attack.push(ms);
        }
    }
    emit(
        "isolation",
        "victim.quiet.p50_ms",
        percentile(&mut victim_quiet, 0.5),
    );
    emit(
        "isolation",
        "victim.quiet.p99_ms",
        percentile(&mut victim_quiet, 0.99),
    );
    emit(
        "isolation",
        "victim.attack.p50_ms",
        percentile(&mut victim_attack, 0.5),
    );
    emit(
        "isolation",
        "victim.attack.p99_ms",
        percentile(&mut victim_attack, 0.99),
    );
    emit("isolation", "victim.attack.max_ms", max(&victim_attack));
    emit(
        "isolation",
        "victim.attack.samples",
        victim_attack.len() as f64,
    );
    emit(
        "isolation",
        "cmd.quiet.p50_ms",
        percentile(&mut cmd_quiet, 0.5),
    );
    emit(
        "isolation",
        "cmd.quiet.p99_ms",
        percentile(&mut cmd_quiet, 0.99),
    );
    emit(
        "isolation",
        "cmd.attack.p50_ms",
        percentile(&mut cmd_attack, 0.5),
    );
    emit(
        "isolation",
        "cmd.attack.p99_ms",
        percentile(&mut cmd_attack, 0.99),
    );
    emit("isolation", "cmd.attack.max_ms", max(&cmd_attack));
    emit("isolation", "cmd.attack.samples", cmd_attack.len() as f64);
    emit("isolation", "attack.connections", attempts as f64);
    emit(
        "isolation",
        "attack.create.p50_ms",
        percentile(&mut creates, 0.5),
    );
    emit("isolation", "attack.create.max_ms", max(&creates));
    emit(
        "isolation",
        "attack.spin.p50_ms",
        percentile(&mut spins, 0.5),
    );
    emit("isolation", "attack.spin.max_ms", max(&spins));
    emit("isolation", "attack.cpu_cores", cpu / attack.as_secs_f64());
    emit("isolation", "victim.lost_faultlab", lost_faultlab as f64);
    emit("isolation", "victim.lost_modify", lost_modify as f64);
    emit("isolation", "threads.peak", peak_threads);
}
