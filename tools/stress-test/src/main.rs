use clap::{Parser, Subcommand};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

// ─── Minecraft Protocol Helpers ──────────────────────────────────────────────

fn write_varint(value: i32) -> Vec<u8> {
    let mut val = value as u32;2
    let mut buf = Vec::with_capacity(5);
    loop {
        if (val & !0x7F) == 0 {
            buf.push(val as u8);
            break;
        }
        buf.push((val & 0x7F | 0x80) as u8);
        val >>= 7;
    }
    buf
}

async fn read_varint(stream: &mut TcpStream) -> io::Result<i32> {
    let mut result: i32 = 0;
    let mut shift = 0u32;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        let b = byte[0];
        result |= ((b & 0x7F) as i32) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "VarInt too big"));
        }
    }
    Ok(result)
}

fn build_packet(packet_id: i32, data: &[u8]) -> Vec<u8> {
    let id_bytes = write_varint(packet_id);
    let length = id_bytes.len() + data.len();
    let length_prefix = write_varint(length as i32);
    let mut packet = Vec::with_capacity(length_prefix.len() + length);
    packet.extend_from_slice(&length_prefix);
    packet.extend_from_slice(&id_bytes);
    packet.extend_from_slice(data);
    packet
}

fn build_handshake(host: &str, port: u16, protocol_version: i32) -> Vec<u8> {
    let mut data = Vec::new();
    // Protocol version
    data.extend_from_slice(&write_varint(protocol_version));
    // Server address (VarInt length prefix + UTF-8 bytes)
    let host_bytes = host.as_bytes();
    data.extend_from_slice(&write_varint(host_bytes.len() as i32));
    data.extend_from_slice(host_bytes);
    // Server port (u16 big-endian)
    data.extend_from_slice(&port.to_be_bytes());
    // Next state = 1 (Status)
    data.extend_from_slice(&write_varint(1));
    build_packet(0x00, &data)
}

fn build_status_request() -> Vec<u8> {
    build_packet(0x00, &[])
}

// ─── CLI ─────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "infrarust-stress-test", about = "Stress test tool for Infrarust Minecraft proxy")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pure SLP flood stress test
    Flood {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 25565)]
        port: u16,
        #[arg(long, default_value_t = 200)]
        concurrency: u32,
        #[arg(long, default_value_t = 300)]
        duration: u64,
    },
    /// Malformed packet stress test
    Malformed {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 25565)]
        port: u16,
        #[arg(long, default_value_t = 50)]
        concurrency: u32,
        #[arg(long, default_value_t = 300)]
        duration: u64,
    },
    /// Mixed flood + malformed
    Mixed {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 25565)]
        port: u16,
        #[arg(long, default_value_t = 200)]
        concurrency: u32,
        #[arg(long, default_value_t = 300)]
        duration: u64,
    },
}

// ─── Shared Stats ────────────────────────────────────────────────────────────

struct Stats {
    success: AtomicU64,
    errors: AtomicU64,
    conn_fail: AtomicU64,
    active: AtomicU64,
    latencies: Mutex<Vec<u64>>,
    all_latencies: Mutex<Vec<u64>>,
}

impl Stats {
    fn new() -> Self {
        Self {
            success: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            conn_fail: AtomicU64::new(0),
            active: AtomicU64::new(0),
            latencies: Mutex::new(Vec::with_capacity(4096)),
            all_latencies: Mutex::new(Vec::with_capacity(65536)),
        }
    }

    fn record_latency(&self, latency_ms: u64) {
        if let Ok(mut v) = self.latencies.lock() {
            v.push(latency_ms);
        }
        if let Ok(mut v) = self.all_latencies.lock() {
            v.push(latency_ms);
        }
    }

    fn drain_latencies(&self) -> Vec<u64> {
        if let Ok(mut v) = self.latencies.lock() {
            std::mem::take(&mut *v)
        } else {
            Vec::new()
        }
    }
}

// ─── Workers ─────────────────────────────────────────────────────────────────

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL_VERSION: i32 = 767; // 1.21.x

async fn do_slp(host: &str, port: u16, stats: &Stats) {
    stats.active.fetch_add(1, Ordering::Relaxed);
    let start = Instant::now();

    let result = timeout(CONNECT_TIMEOUT, async {
        let addr = format!("{host}:{port}");
        let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
            stats.conn_fail.fetch_add(1, Ordering::Relaxed);
            e
        })?;
        let _ = stream.set_nodelay(true);

        // Send handshake
        let handshake = build_handshake(host, port, PROTOCOL_VERSION);
        stream.write_all(&handshake).await?;

        // Send status request
        let status_req = build_status_request();
        stream.write_all(&status_req).await?;

        // Read response: VarInt length, then payload
        let length = read_varint(&mut stream).await?;
        if length <= 0 || length > 1_048_576 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad response length"));
        }
        // Read at least 1 byte to confirm we got data
        let mut buf = vec![0u8; std::cmp::min(length as usize, 4096)];
        stream.read_exact(&mut buf).await?;

        // Drain remaining if any
        let remaining = length as usize - buf.len();
        if remaining > 0 {
            let mut drain = vec![0u8; std::cmp::min(remaining, 8192)];
            // Best-effort drain, don't care if it fails
            let _ = stream.read(&mut drain).await;
        }

        Ok::<(), io::Error>(())
    })
    .await;

    let elapsed = start.elapsed().as_millis() as u64;
    stats.active.fetch_sub(1, Ordering::Relaxed);

    match result {
        Ok(Ok(())) => {
            stats.success.fetch_add(1, Ordering::Relaxed);
            stats.record_latency(elapsed);
        }
        Ok(Err(_)) | Err(_) => {
            stats.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn flood_worker(host: String, port: u16, stats: Arc<Stats>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        do_slp(&host, port, &stats).await;
    }
}

#[derive(Clone, Copy)]
enum MalformedType {
    HugeHostname,
    BogusVarIntLength,
    RandomBytes,
    EarlyClose,
    RandomAfterHandshake,
    Slowloris,
}

const MALFORMED_TYPES: [MalformedType; 6] = [
    MalformedType::HugeHostname,
    MalformedType::BogusVarIntLength,
    MalformedType::RandomBytes,
    MalformedType::EarlyClose,
    MalformedType::RandomAfterHandshake,
    MalformedType::Slowloris,
];

async fn do_malformed(host: &str, port: u16, variant: MalformedType, stats: &Stats) {
    stats.active.fetch_add(1, Ordering::Relaxed);
    let start = Instant::now();

    let result = timeout(CONNECT_TIMEOUT, async {
        let addr = format!("{host}:{port}");
        let mut stream = TcpStream::connect(&addr).await.map_err(|e| {
            stats.conn_fail.fetch_add(1, Ordering::Relaxed);
            e
        })?;
        let _ = stream.set_nodelay(true);

        match variant {
            MalformedType::HugeHostname => {
                // Handshake with 50,000 character hostname
                let huge_host = "A".repeat(50_000);
                let pkt = build_handshake(&huge_host, port, PROTOCOL_VERSION);
                stream.write_all(&pkt).await?;
                let status_req = build_status_request();
                stream.write_all(&status_req).await?;
                // Try to read response
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf).await;
            }
            MalformedType::BogusVarIntLength => {
                // Send a VarInt claiming 2GB length
                // VarInt for 2147483647 (0x7FFFFFFF)
                let bogus: [u8; 5] = [0xFF, 0xFF, 0xFF, 0xFF, 0x07];
                stream.write_all(&bogus).await?;
                // Send some garbage after
                let garbage = [0xDE, 0xAD, 0xBE, 0xEF];
                stream.write_all(&garbage).await?;
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf).await;
            }
            MalformedType::RandomBytes => {
                // Send 256 random-ish bytes and close
                let garbage: Vec<u8> = (0..256).map(|i| (i * 37 + 13) as u8).collect();
                stream.write_all(&garbage).await?;
            }
            MalformedType::EarlyClose => {
                // Valid handshake, then close immediately (no status request)
                let handshake = build_handshake(host, port, PROTOCOL_VERSION);
                stream.write_all(&handshake).await?;
                // Drop / close immediately
            }
            MalformedType::RandomAfterHandshake => {
                // Valid handshake, then random bytes instead of status request
                let handshake = build_handshake(host, port, PROTOCOL_VERSION);
                stream.write_all(&handshake).await?;
                let garbage: Vec<u8> = (0..128).map(|i| (i * 53 + 7) as u8).collect();
                stream.write_all(&garbage).await?;
                let mut buf = [0u8; 64];
                let _ = stream.read(&mut buf).await;
            }
            MalformedType::Slowloris => {
                // Connect and do nothing (hold connection open)
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        }

        Ok::<(), io::Error>(())
    })
    .await;

    let elapsed = start.elapsed().as_millis() as u64;
    stats.active.fetch_sub(1, Ordering::Relaxed);

    match result {
        Ok(Ok(())) => {
            stats.success.fetch_add(1, Ordering::Relaxed);
            stats.record_latency(elapsed);
        }
        Ok(Err(_)) | Err(_) => {
            stats.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn malformed_worker(
    host: String,
    port: u16,
    variant: MalformedType,
    stats: Arc<Stats>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        do_malformed(&host, port, variant, &stats).await;
    }
}

// ─── Reporter ────────────────────────────────────────────────────────────────

async fn reporter(stats: Arc<Stats>, stop: Arc<AtomicBool>, start_time: Instant) {
    let mut prev_success = 0u64;
    let mut prev_errors = 0u64;
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.tick().await; // skip first immediate tick

    while !stop.load(Ordering::Relaxed) {
        interval.tick().await;

        let elapsed = start_time.elapsed().as_secs();
        let total_success = stats.success.load(Ordering::Relaxed);
        let total_errors = stats.errors.load(Ordering::Relaxed);
        let total_conn_fail = stats.conn_fail.load(Ordering::Relaxed);
        let active = stats.active.load(Ordering::Relaxed);

        let delta_success = total_success - prev_success;
        let delta_errors = total_errors - prev_errors;
        let rate_success = delta_success as f64 / 5.0;
        let rate_errors = delta_errors as f64 / 5.0;

        prev_success = total_success;
        prev_errors = total_errors;

        let latencies = stats.drain_latencies();
        let avg_latency = if latencies.is_empty() {
            0
        } else {
            latencies.iter().sum::<u64>() / latencies.len() as u64
        };

        let mins = elapsed / 60;
        let secs = elapsed % 60;
        println!(
            "[{mins:02}:{secs:02}] OK: {total_success} ({rate_success:.1}/s) | ERR: {total_errors} ({rate_errors:.1}/s) | CONN_FAIL: {total_conn_fail} | AVG_LATENCY: {avg_latency}ms | ACTIVE: {active}"
        );
    }
}

fn print_summary(stats: &Stats, duration: Duration) {
    let total_secs = duration.as_secs();
    let total_success = stats.success.load(Ordering::Relaxed);
    let total_errors = stats.errors.load(Ordering::Relaxed);
    let total_conn_fail = stats.conn_fail.load(Ordering::Relaxed);

    let all_latencies = stats.all_latencies.lock().unwrap_or_else(|e| e.into_inner());

    let (avg_latency, p99_latency) = if all_latencies.is_empty() {
        (0, 0)
    } else {
        let avg = all_latencies.iter().sum::<u64>() / all_latencies.len() as u64;
        let mut sorted = all_latencies.clone();
        sorted.sort_unstable();
        let p99_idx = (sorted.len() as f64 * 0.99).ceil() as usize - 1;
        let p99 = sorted[p99_idx.min(sorted.len() - 1)];
        (avg, p99)
    };

    let throughput = if total_secs > 0 {
        total_success as f64 / total_secs as f64
    } else {
        0.0
    };

    println!();
    println!("=== Résultats ===");
    println!("Durée totale   : {total_secs}s");
    println!("Total succès   : {total_success}");
    println!("Total erreurs  : {total_errors}");
    println!("Total conn fail: {total_conn_fail}");
    println!("Latence avg    : {avg_latency}ms");
    println!("Latence p99    : {p99_latency}ms");
    println!("Débit moyen    : {throughput:.1}/s");
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let (host, port, concurrency, duration_secs, mode) = match &cli.command {
        Command::Flood { host, port, concurrency, duration } => {
            (host.clone(), *port, *concurrency, *duration, "flood")
        }
        Command::Malformed { host, port, concurrency, duration } => {
            (host.clone(), *port, *concurrency, *duration, "malformed")
        }
        Command::Mixed { host, port, concurrency, duration } => {
            (host.clone(), *port, *concurrency, *duration, "mixed")
        }
    };

    println!("=== Infrarust Stress Test ===");
    println!("Mode       : {mode}");
    println!("Target     : {host}:{port}");
    println!("Concurrency: {concurrency}");
    println!("Duration   : {duration_secs}s");
    println!();

    let stats = Arc::new(Stats::new());
    let stop = Arc::new(AtomicBool::new(false));
    let start_time = Instant::now();

    // Spawn reporter
    let reporter_handle = {
        let stats = Arc::clone(&stats);
        let stop = Arc::clone(&stop);
        tokio::spawn(reporter(stats, stop, start_time))
    };

    // Spawn workers
    let mut handles = Vec::new();

    match mode {
        "flood" => {
            for _ in 0..concurrency {
                let h = host.clone();
                let s = Arc::clone(&stats);
                let st = Arc::clone(&stop);
                handles.push(tokio::spawn(flood_worker(h, port, s, st)));
            }
        }
        "malformed" => {
            let per_type = std::cmp::max(1, concurrency as usize / MALFORMED_TYPES.len());
            for variant in &MALFORMED_TYPES {
                for _ in 0..per_type {
                    let h = host.clone();
                    let s = Arc::clone(&stats);
                    let st = Arc::clone(&stop);
                    let v = *variant;
                    handles.push(tokio::spawn(malformed_worker(h, port, v, s, st)));
                }
            }
        }
        "mixed" => {
            let flood_count = (concurrency as f64 * 0.7) as u32;
            let malformed_count = concurrency - flood_count;

            for _ in 0..flood_count {
                let h = host.clone();
                let s = Arc::clone(&stats);
                let st = Arc::clone(&stop);
                handles.push(tokio::spawn(flood_worker(h, port, s, st)));
            }

            let per_type = std::cmp::max(1, malformed_count as usize / MALFORMED_TYPES.len());
            for variant in &MALFORMED_TYPES {
                for _ in 0..per_type {
                    let h = host.clone();
                    let s = Arc::clone(&stats);
                    let st = Arc::clone(&stop);
                    let v = *variant;
                    handles.push(tokio::spawn(malformed_worker(h, port, v, s, st)));
                }
            }
        }
        _ => unreachable!(),
    }

    // Wait for duration or Ctrl+C
    let stop_clone = Arc::clone(&stop);
    let duration = Duration::from_secs(duration_secs);
    tokio::select! {
        _ = tokio::time::sleep(duration) => {
            println!("\nDurée écoulée, arrêt en cours...");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCtrl+C reçu, arrêt en cours...");
        }
    }
    stop_clone.store(true, Ordering::Relaxed);

    // Wait for workers to finish (with a timeout so we don't hang)
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        for h in handles {
            let _ = h.await;
        }
    })
    .await;

    // Stop reporter
    let _ = reporter_handle.await;

    // Print summary
    let actual_duration = start_time.elapsed();
    print_summary(&stats, actual_duration);
}
