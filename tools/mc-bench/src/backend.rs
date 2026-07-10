//! `serve-backend` — a minimal mock Minecraft backend used as the benchmark sink.
//!
//! Targets protocol versions < 1.20.2 (764): after `CLoginSuccess` both sides go
//! straight to Play with NO config phase, which keeps the benchmark path minimal.

use std::sync::Arc;

use clap::Args;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::{DecodedPacket, build_default_registry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
use infrarust_protocol::{CLoginSuccess, CSetCompression, PacketRegistry, SHandshake, VarInt};
use tokio::net::{TcpListener, TcpStream};

use crate::proto::{FramedConn, PING_CLIENTBOUND_ID, PING_SERVERBOUND_ID, snap_to_supported};

#[derive(Args, Debug)]
pub struct BackendArgs {
    /// Address to bind the mock backend to.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Port to bind the mock backend to.
    #[arg(long, default_value_t = 25566)]
    port: u16,
    /// Optional compression threshold. When set, a `CSetCompression` is sent
    /// before `CLoginSuccess` and all later packets use the compressed format.
    #[arg(long)]
    compression: Option<i32>,
}

pub async fn run(args: BackendArgs) -> std::io::Result<()> {
    let registry = Arc::new(build_default_registry());
    let listener = TcpListener::bind((args.host.as_str(), args.port)).await?;
    println!(
        "mc-bench backend listening on {}:{} (compression: {:?})",
        args.host, args.port, args.compression
    );

    loop {
        let (stream, _peer) = listener.accept().await?;
        let registry = Arc::clone(&registry);
        let compression = args.compression;
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, &registry, compression).await {
                eprintln!("backend connection error: {e}");
            }
        });
    }
}

async fn handle_conn(
    stream: TcpStream,
    registry: &PacketRegistry,
    compression: Option<i32>,
) -> std::io::Result<()> {
    stream.set_nodelay(true)?;
    let mut conn = FramedConn::new(stream);

    // 1. Handshake — extract the client's advertised protocol version.
    let hs_frame = conn
        .read_frame()
        .await?
        .ok_or_else(|| eof("closed before handshake"))?;
    let proto = match registry.decode_frame(
        &hs_frame,
        ConnectionState::Handshake,
        Direction::Serverbound,
        ProtocolVersion::V1_8,
    ) {
        Ok(DecodedPacket::Typed { packet, .. }) => packet
            .as_any()
            .downcast_ref::<SHandshake>()
            .map_or(ProtocolVersion::V1_18_2.0, |hs| hs.protocol_version.0),
        _ => ProtocolVersion::V1_18_2.0,
    };
    if proto >= ProtocolVersion::V1_20_2.0 {
        return Err(invalid(format!(
            "protocol {proto} >= 764 unsupported; this mock targets < 1.20.2 (no config phase)"
        )));
    }
    let version = snap_to_supported(proto);

    // 2. LoginStart — we only need the username for the success reply.
    let login_frame = conn
        .read_frame()
        .await?
        .ok_or_else(|| eof("closed before login start"))?;
    let username = match registry.decode_frame(
        &login_frame,
        ConnectionState::Login,
        Direction::Serverbound,
        version,
    ) {
        Ok(DecodedPacket::Typed { packet, .. }) => packet
            .as_any()
            .downcast_ref::<infrarust_protocol::SLoginStart>()
            .map_or_else(|| "player".to_string(), |ls| ls.name.clone()),
        _ => "player".to_string(),
    };

    // 3. Optional CSetCompression, then CLoginSuccess.
    if let Some(threshold) = compression {
        let id = registry
            .get_packet_id::<CSetCompression>(
                ConnectionState::Login,
                Direction::Clientbound,
                version,
            )
            .unwrap_or(0x03);
        let mut payload = Vec::new();
        CSetCompression {
            threshold: VarInt(threshold),
        }
        .encode(&mut payload, version)
        .map_err(|e| invalid(e.to_string()))?;
        conn.write_frame(id, &payload).await?;
        conn.set_compression(threshold);
    }

    let success = CLoginSuccess {
        uuid: offline_uuid(&username),
        username,
        properties: Vec::new(),
        strict_error_handling: false,
    };
    let success_id = registry
        .get_packet_id::<CLoginSuccess>(ConnectionState::Login, Direction::Clientbound, version)
        .unwrap_or(0x02);
    let mut payload = Vec::new();
    success
        .encode(&mut payload, version)
        .map_err(|e| invalid(e.to_string()))?;
    conn.write_frame(success_id, &payload).await?;

    // 4. Version < 764 → straight to Play. Echo benchmark pings, ignore the rest.
    loop {
        let frame = match conn.read_frame().await? {
            Some(f) => f,
            None => return Ok(()),
        };
        if frame.id == PING_SERVERBOUND_ID && frame.payload.len() >= 8 {
            // Echo the full payload so the clientbound direction carries the
            // same size (and crosses the compression threshold when the
            // serverbound ping does).
            conn.write_frame(PING_CLIENTBOUND_ID, &frame.payload).await?;
        }
    }
}

/// Deterministic offline-mode UUID, mirroring `OfflinePlayer:<name>` MD5 (v3).
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
