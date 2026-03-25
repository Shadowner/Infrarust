use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use infrarust_protocol::codec::VarInt;
use infrarust_protocol::io::decoder::PacketDecoder;
use infrarust_protocol::io::encoder::PacketEncoder;
use infrarust_protocol::io::frame::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{
    CFinishConfig, CKnownPacks, SAcknowledgeFinishConfig, SKnownPacks,
};
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged, SLoginStart,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry, build_default_registry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use infrarust_core::registry_data::extractor_format::{ExtractedRegistryData, FrameData};

/// Connects to a Minecraft server and captures config-phase registry data.
pub async fn extract_registry_data(
    server_addr: &str,
    username: &str,
    protocol_version_override: Option<i32>,
) -> anyhow::Result<ExtractedRegistryData> {
    let protocol_version = protocol_version_override
        .map(ProtocolVersion)
        .unwrap_or(ProtocolVersion::V1_21_11);

    tracing::info!(
        server = server_addr,
        protocol = protocol_version.0,
        version_name = protocol_version.name(),
        "Connecting to server"
    );

    let mut stream = TcpStream::connect(server_addr).await?;
    let registry = build_default_registry();
    let mut decoder = PacketDecoder::new();
    let mut encoder = PacketEncoder::new();

    let (host, port) = parse_addr(server_addr);
    let handshake = SHandshake {
        protocol_version: VarInt(protocol_version.0),
        server_address: host.to_string(),
        server_port: port,
        next_state: ConnectionState::Login,
    };
    send_packet(
        &mut stream,
        &mut encoder,
        &handshake,
        &registry,
        ConnectionState::Handshake,
        Direction::Serverbound,
        protocol_version,
    )
    .await?;
    tracing::debug!("Handshake sent");

    let login_start = SLoginStart {
        name: username.to_string(),
        uuid: Some(offline_uuid(username)),
        signature_data: None,
    };
    send_packet(
        &mut stream,
        &mut encoder,
        &login_start,
        &registry,
        ConnectionState::Login,
        Direction::Serverbound,
        protocol_version,
    )
    .await?;
    tracing::debug!("LoginStart sent for '{username}'");

    let mut state = ConnectionState::Login;
    let mut collected_registry_frames: Vec<FrameData> = Vec::new();
    let mut collected_known_packs: Option<FrameData> = None;
    let mut buf = BytesMut::with_capacity(8192);

    loop {
        let n = stream.read_buf(&mut buf).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Server closed connection"));
        }

        decoder.queue_bytes(&buf.split_to(buf.len()));

        while let Some(frame) = decoder.try_next_frame()? {
            match state {
                ConnectionState::Login => {
                    handle_login_frame(
                        &frame,
                        &registry,
                        protocol_version,
                        &mut state,
                        &mut stream,
                        &mut encoder,
                        &mut decoder,
                    )
                    .await?;
                }
                ConnectionState::Config => {
                    let done = handle_config_frame(
                        &frame,
                        &registry,
                        protocol_version,
                        &mut stream,
                        &mut encoder,
                        &mut collected_registry_frames,
                        &mut collected_known_packs,
                    )
                    .await?;

                    if done {
                        tracing::info!(
                            registry_frames = collected_registry_frames.len(),
                            has_known_packs = collected_known_packs.is_some(),
                            "Config phase complete"
                        );

                        return Ok(ExtractedRegistryData {
                            format_version: 1,
                            protocol_version: protocol_version.0,
                            minecraft_version: protocol_version.name().to_string(),
                            extraction_date: chrono::Utc::now().to_rfc3339(),
                            known_packs_frame: collected_known_packs,
                            registry_frames: collected_registry_frames,
                        });
                    }
                }
                other => {
                    return Err(anyhow::anyhow!("Unexpected state: {other:?}"));
                }
            }
        }
    }
}

async fn handle_login_frame(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
    state: &mut ConnectionState,
    stream: &mut TcpStream,
    encoder: &mut PacketEncoder,
    decoder: &mut PacketDecoder,
) -> anyhow::Result<()> {
    let decoded = registry.decode_frame(
        frame,
        ConnectionState::Login,
        Direction::Clientbound,
        version,
    )?;

    match decoded {
        DecodedPacket::Typed { packet, .. } => {
            if let Some(compression) = packet.as_any().downcast_ref::<CSetCompression>() {
                tracing::debug!(threshold = compression.threshold.0, "Compression enabled");
                decoder.set_compression(compression.threshold.0);
                encoder.set_compression(compression.threshold.0);
                return Ok(());
            }

            if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                tracing::info!("LoginSuccess received, sending LoginAcknowledged");

                if version.no_less_than(ProtocolVersion::V1_20_2) {
                    send_packet(
                        stream,
                        encoder,
                        &SLoginAcknowledged,
                        registry,
                        ConnectionState::Login,
                        Direction::Serverbound,
                        version,
                    )
                    .await?;
                    *state = ConnectionState::Config;
                    tracing::info!("Transitioned to Config state");
                } else {
                    return Err(anyhow::anyhow!(
                        "Protocol version {} does not have config phase (requires >= 1.20.2)",
                        version.0
                    ));
                }
                return Ok(());
            }

            if let Some(disconnect) = packet.as_any().downcast_ref::<CLoginDisconnect>() {
                return Err(anyhow::anyhow!(
                    "Server rejected login: {}",
                    disconnect.reason
                ));
            }

            tracing::warn!("Unexpected login packet: {}", packet.packet_name());
        }
        DecodedPacket::Opaque { id, .. } => {
            tracing::warn!(packet_id = id, "Unknown login packet (opaque)");
        }
    }

    Ok(())
}

/// Returns `true` when config phase is complete (CFinishConfig received).
async fn handle_config_frame(
    frame: &PacketFrame,
    registry: &PacketRegistry,
    version: ProtocolVersion,
    stream: &mut TcpStream,
    encoder: &mut PacketEncoder,
    registry_frames: &mut Vec<FrameData>,
    known_packs: &mut Option<FrameData>,
) -> anyhow::Result<bool> {
    let known_packs_id = registry.get_packet_id::<CKnownPacks>(
        ConnectionState::Config,
        Direction::Clientbound,
        version,
    );
    let finish_config_id = registry.get_packet_id::<CFinishConfig>(
        ConnectionState::Config,
        Direction::Clientbound,
        version,
    );

    if finish_config_id == Some(frame.id) {
        tracing::debug!("CFinishConfig received, sending SAcknowledgeFinishConfig");
        send_packet(
            stream,
            encoder,
            &SAcknowledgeFinishConfig,
            registry,
            ConnectionState::Config,
            Direction::Serverbound,
            version,
        )
        .await?;
        return Ok(true);
    }

    if known_packs_id == Some(frame.id) {
        tracing::debug!(payload_len = frame.payload.len(), "CKnownPacks captured");
        *known_packs = Some(FrameData {
            packet_id: frame.id,
            payload: frame.payload.to_vec(),
        });

        let response = SKnownPacks { packs: vec![] };
        send_packet(
            stream,
            encoder,
            &response,
            registry,
            ConnectionState::Config,
            Direction::Serverbound,
            version,
        )
        .await?;
        tracing::debug!("SKnownPacks (empty) sent");
        return Ok(false);
    }

    tracing::debug!(
        packet_id = frame.id,
        payload_len = frame.payload.len(),
        total = registry_frames.len() + 1,
        "Config frame captured"
    );
    registry_frames.push(FrameData {
        packet_id: frame.id,
        payload: frame.payload.to_vec(),
    });

    Ok(false)
}

async fn send_packet<P: Packet + 'static>(
    stream: &mut TcpStream,
    encoder: &mut PacketEncoder,
    packet: &P,
    registry: &PacketRegistry,
    state: ConnectionState,
    direction: Direction,
    version: ProtocolVersion,
) -> anyhow::Result<()> {
    let packet_id = registry
        .get_packet_id::<P>(state, direction, version)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No packet ID for {} in {state:?}/{direction:?}/{}",
                P::NAME,
                version.0,
            )
        })?;

    let mut payload = Vec::new();
    packet.encode(&mut payload, version)?;

    encoder.append_raw(packet_id, &payload)?;
    let bytes = encoder.take();
    stream.write_all(&bytes).await?;
    stream.flush().await?;

    Ok(())
}

fn offline_uuid(username: &str) -> uuid::Uuid {
    let input = format!("OfflinePlayer:{username}");
    uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_URL, input.as_bytes())
}

fn parse_addr(addr: &str) -> (&str, u16) {
    if let Some((host, port_str)) = addr.rsplit_once(':') {
        let port = port_str.parse().unwrap_or(25565);
        (host, port)
    } else {
        (addr, 25565)
    }
}
