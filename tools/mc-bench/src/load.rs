//! `load` — open-loop, constant-arrival-rate Play-state load generator.
//!
//! Sends benchmark pings on a fixed wall-clock grid so the offered load stays
//! constant no matter how the proxy responds. That open-loop grid is what avoids
//! the closed-loop coordinated-omission trap (the generator never waits on an
//! echo before sending the next ping). Each ping is stamped with its send time
//! and compared against its echo to get round-trip latency.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use clap::Args;
use hdrhistogram::Histogram;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::build_default_registry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{CLoginSuccess, PacketRegistry, SHandshake, SLoginStart, VarInt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;
use tokio::time::sleep_until;

use crate::proto::{FramedConn, PING_CLIENTBOUND_ID, PING_SERVERBOUND_ID, snap_to_supported};

#[derive(Args, Debug, Clone)]
pub struct LoadArgs {
    /// Target host (the proxy's port, or the backend's port for the baseline).
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Target port.
    #[arg(long, default_value_t = 25566)]
    port: u16,
    /// Hostname placed in the Handshake so the proxy routes correctly.
    #[arg(long, default_value = "127.0.0.1")]
    server_address: String,
    /// Protocol version to advertise. Must be < 764 (1.20.2) — no config phase.
    #[arg(long, default_value_t = 758)]
    protocol: i32,
    /// Number of concurrent worker connections.
    #[arg(long, default_value_t = 100)]
    concurrency: usize,
    /// Ping send rate per connection, in hertz.
    #[arg(long, default_value_t = 20)]
    rate: u64,
    /// Measurement duration in seconds (after warmup).
    #[arg(long, default_value_t = 30)]
    duration: u64,
    /// Warmup duration in seconds (not recorded).
    #[arg(long, default_value_t = 5)]
    warmup: u64,
}

/// One latency sample reported by a worker, in microseconds.
struct Sample(u64);

#[derive(Default)]
struct Counters {
    conn_ok: AtomicU64,
    conn_fail: AtomicU64,
    sent: AtomicU64,
    echoed: AtomicU64,
}

/// Nanoseconds from `base` to now.
fn elapsed_ns(base: TokioInstant) -> u64 {
    TokioInstant::now().saturating_duration_since(base).as_nanos() as u64
}

pub async fn run(args: LoadArgs) -> std::io::Result<()> {
    if args.protocol >= ProtocolVersion::V1_20_2.0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "protocol {} must be < 764 (1.20.2); newer versions need a config phase",
                args.protocol
            ),
        ));
    }
    if args.rate == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "rate must be > 0",
        ));
    }

    let registry = Arc::new(build_default_registry());
    let counters = Arc::new(Counters::default());

    let (tx, mut rx) = mpsc::unbounded_channel::<Sample>();

    println!(
        "mc-bench load → {}:{} (server_address={}, protocol={}, concurrency={}, rate={}hz, warmup={}s, duration={}s)",
        args.host,
        args.port,
        args.server_address,
        args.protocol,
        args.concurrency,
        args.rate,
        args.warmup,
        args.duration,
    );

    // Aligned start so every worker shares one warmup/measurement window.
    // The 500ms lead-in lets all connections finish login before the grid begins.
    let grid_start = TokioInstant::now() + Duration::from_millis(500);
    let warmup_until = grid_start + Duration::from_secs(args.warmup);
    let stop_at = warmup_until + Duration::from_secs(args.duration);

    let mut handles = Vec::with_capacity(args.concurrency);
    for i in 0..args.concurrency {
        let args = args.clone();
        let registry = Arc::clone(&registry);
        let counters = Arc::clone(&counters);
        let tx = tx.clone();
        handles.push(tokio::spawn(async move {
            run_worker(
                i,
                args,
                registry,
                counters,
                grid_start,
                warmup_until,
                stop_at,
                tx,
            )
            .await;
        }));
    }
    drop(tx);

    // Drain samples into the aggregate histogram while workers run.
    let mut hist: Histogram<u64> =
        Histogram::new_with_bounds(1, 60_000_000, 3).expect("histogram bounds");
    while let Some(Sample(us)) = rx.recv().await {
        hist.saturating_record(us);
    }
    for h in handles {
        let _ = h.await;
    }

    report(&hist, &counters, args.duration);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    index: usize,
    args: LoadArgs,
    registry: Arc<PacketRegistry>,
    counters: Arc<Counters>,
    grid_start: TokioInstant,
    warmup_until: TokioInstant,
    stop_at: TokioInstant,
    tx: mpsc::UnboundedSender<Sample>,
) {
    let conn = match connect_and_login(index, &args, &registry).await {
        Ok(conn) => {
            counters.conn_ok.fetch_add(1, Ordering::Relaxed);
            conn
        }
        Err(e) => {
            counters.conn_fail.fetch_add(1, Ordering::Relaxed);
            if index == 0 {
                eprintln!("worker {index} login failed: {e}");
            }
            return;
        }
    };

    let (mut reader, mut writer) = conn.into_split();

    // Reader task: each echo carries the send time (ns since grid_start) stamped
    // into it; round-trip latency is the elapsed time now minus that token.
    let counters_r = Arc::clone(&counters);
    let reader_task = tokio::spawn(async move {
        while let Ok(Some(frame)) = reader.read_frame().await {
            if frame.id == PING_CLIENTBOUND_ID && frame.payload.len() >= 8 {
                let token = u64::from_be_bytes(frame.payload[..8].try_into().unwrap_or_default());
                if token == 0 {
                    continue; // warmup ping, not recorded
                }
                let us = elapsed_ns(grid_start).saturating_sub(token) / 1000;
                counters_r.echoed.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(Sample(us.max(1)));
            }
        }
    });

    // Sender: a fixed wall-clock grid keeps the offered load constant (open-loop).
    // `recording` is decided on the grid instant; the stamped token is the actual
    // send time, so latency reflects proxy/backend service time, not the
    // generator's own timer granularity. Warmup pings carry 0 and are skipped.
    let interval = Duration::from_nanos(1_000_000_000 / args.rate);
    let mut next = grid_start;
    while next < stop_at {
        sleep_until(next).await;
        let recording = next >= warmup_until;
        let token = if recording { elapsed_ns(grid_start).max(1) } else { 0 };
        let payload = token.to_be_bytes();
        if writer
            .write_frame(PING_SERVERBOUND_ID, &payload)
            .await
            .is_err()
        {
            break;
        }
        if recording {
            counters.sent.fetch_add(1, Ordering::Relaxed);
        }
        next += interval;
    }

    // Allow late echoes to land before tearing down the reader.
    let _ = tokio::time::timeout(Duration::from_millis(500), reader_task).await;
}

async fn connect_and_login(
    index: usize,
    args: &LoadArgs,
    registry: &PacketRegistry,
) -> std::io::Result<FramedConn> {
    let version = snap_to_supported(args.protocol);
    let stream = TcpStream::connect((args.host.as_str(), args.port)).await?;
    stream.set_nodelay(true)?;
    let mut conn = FramedConn::new(stream);

    // Handshake (next_state = Login).
    let handshake = SHandshake {
        protocol_version: VarInt(args.protocol),
        server_address: args.server_address.clone(),
        server_port: args.port,
        next_state: ConnectionState::Login,
    };
    let hs_id = registry
        .get_packet_id::<SHandshake>(ConnectionState::Handshake, Direction::Serverbound, version)
        .unwrap_or(0x00);
    let mut payload = Vec::new();
    handshake
        .encode(&mut payload, version)
        .map_err(|e| invalid(e.to_string()))?;
    conn.write_frame(hs_id, &payload).await?;

    // LoginStart with an offline-style identity.
    let name = format!("bench_{index}");
    let login = SLoginStart {
        uuid: Some(offline_uuid(&name)),
        name,
        signature_data: None,
    };
    let login_id = registry
        .get_packet_id::<SLoginStart>(ConnectionState::Login, Direction::Serverbound, version)
        .unwrap_or(0x00);
    let mut payload = Vec::new();
    login
        .encode(&mut payload, version)
        .map_err(|e| invalid(e.to_string()))?;
    conn.write_frame(login_id, &payload).await?;

    // Read clientbound login frames until CLoginSuccess; then we're in Play.
    let success_id = registry
        .get_packet_id::<CLoginSuccess>(ConnectionState::Login, Direction::Clientbound, version)
        .unwrap_or(0x02);
    loop {
        let frame = conn
            .read_frame()
            .await?
            .ok_or_else(|| eof("closed before login success"))?;
        if frame.id == success_id {
            return Ok(conn);
        }
        // CSetCompression — match by typed decode so compression activates.
        if let Ok(infrarust_protocol::registry::DecodedPacket::Typed { packet, .. }) = registry
            .decode_frame(&frame, ConnectionState::Login, Direction::Clientbound, version)
            && let Some(c) = packet
                .as_any()
                .downcast_ref::<infrarust_protocol::CSetCompression>()
        {
            conn.set_compression(c.threshold.0);
        }
    }
}

fn report(hist: &Histogram<u64>, counters: &Counters, duration_secs: u64) {
    let sent = counters.sent.load(Ordering::Relaxed);
    let echoed = counters.echoed.load(Ordering::Relaxed);
    let conn_ok = counters.conn_ok.load(Ordering::Relaxed);
    let conn_fail = counters.conn_fail.load(Ordering::Relaxed);
    let throughput = if duration_secs > 0 {
        echoed as f64 / duration_secs as f64
    } else {
        0.0
    };

    println!("\n=== mc-bench results ===");
    println!("connections:   {conn_ok} ok, {conn_fail} failed");
    println!("pings sent:     {sent}");
    println!("echoes:         {echoed}");
    println!("throughput:     {throughput:.0} echoes/sec");
    if hist.is_empty() {
        println!("latency:        no samples recorded");
        return;
    }
    println!("latency (microseconds):");
    println!("  mean   {:>10.1}", hist.mean());
    println!("  p50    {:>10}", hist.value_at_quantile(0.50));
    println!("  p90    {:>10}", hist.value_at_quantile(0.90));
    println!("  p99    {:>10}", hist.value_at_quantile(0.99));
    println!("  p99.9  {:>10}", hist.value_at_quantile(0.999));
    println!("  max    {:>10}", hist.max());
}

fn offline_uuid(name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v3(
        &uuid::Uuid::NAMESPACE_OID,
        format!("OfflinePlayer:{name}").as_bytes(),
    )
}

fn eof(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, msg)
}

fn invalid(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}
