use std::{io, net::SocketAddr, sync::Arc};

use infrarust_config::models::logging::LogType;
use infrarust_protocol::{
    minecraft::java::{
        handshake::ServerBoundHandshake,
        legacy::{
            handshake::parse_legacy_handshake,
            kick::{build_legacy_kick_beta, build_legacy_kick_v1_4},
            ping::{LegacyPingVariant, parse_legacy_ping},
        },
        status::serverbound_request::SERVERBOUND_REQUEST_ID,
    },
    types::VarInt,
    version::Version,
};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::{
    Connection,
    network::packet::{Packet, PacketCodec},
    server::{
        ServerRequest,
        backend::Server,
        gateway::Gateway,
        motd::{MotdState, generate_legacy_motd_for_state, generate_legacy_motd_from_packet},
    },
};

/// Handle a legacy server list ping (first byte was 0xFE).
///
/// This handles all three legacy ping variants:
/// - Beta 1.8–1.3: `0xFE` only
/// - 1.4–1.5: `0xFE 0x01`
/// - 1.6: `0xFE 0x01 0xFA` + MC|PingHost
pub async fn handle_legacy_ping(
    conn: &mut Connection,
    gateway: &Arc<Gateway>,
    session_id: Uuid,
    client_addr: SocketAddr,
) -> io::Result<()> {
    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Detected legacy ping from {}", client_addr
    );

    // Read all available data from the client (legacy ping is small, at most ~100 bytes)
    let raw_data = read_legacy_ping_data(conn).await?;

    // Parse the variant
    let variant = parse_legacy_ping(&raw_data).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse legacy ping: {}", e),
        )
    })?;

    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Legacy ping variant: {:?}, hostname: {:?}",
        match &variant {
            LegacyPingVariant::Beta => "Beta",
            LegacyPingVariant::V1_4 => "V1_4",
            LegacyPingVariant::V1_6 { .. } => "V1_6",
        },
        variant.hostname()
    );

    // Look up server config by hostname (or use first available for domain-less pings)
    let server_config = if let Some(hostname) = variant.hostname() {
        gateway.find_server(hostname).await
    } else {
        find_default_server(gateway).await
    };

    let response_bytes = match server_config {
        Some(config) => {
            // Tier 1: Forward the raw legacy ping to backend and relay the response.
            // This works with any server (legacy or modern) since all Minecraft
            // servers handle the 0xFE server list ping natively.
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                forward_legacy_ping_to_backend(&raw_data, &config, session_id),
            )
            .await
            {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(e)) => {
                    debug!(
                        log_type = LogType::PacketProcessing.as_str(),
                        "Legacy ping passthrough failed: {}, trying modern fetch", e
                    );
                    // Tier 2: Try modern protocol fetch and convert to legacy format.
                    // Keeps compatibility with backends behind modern-only proxies.
                    match fetch_and_convert_to_legacy(&variant, &config, session_id).await {
                        Ok(bytes) => bytes,
                        Err(e2) => {
                            debug!(
                                log_type = LogType::PacketProcessing.as_str(),
                                "Modern fetch also failed: {}, using fallback", e2
                            );
                            generate_legacy_fallback(&variant, &config)
                        }
                    }
                }
                Err(_) => {
                    debug!(
                        log_type = LogType::PacketProcessing.as_str(),
                        "Legacy ping passthrough timed out, trying modern fetch"
                    );
                    match fetch_and_convert_to_legacy(&variant, &config, session_id).await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            debug!(
                                log_type = LogType::PacketProcessing.as_str(),
                                "Modern fetch also failed: {}, using fallback", e
                            );
                            generate_legacy_fallback(&variant, &config)
                        }
                    }
                }
            }
        }
        None => {
            debug!(
                log_type = LogType::PacketProcessing.as_str(),
                "No server config found for legacy ping, sending fallback"
            );
            generate_legacy_no_server(&variant)
        }
    };

    // Write the legacy response and close
    conn.write_raw(&response_bytes).await?;
    conn.flush().await?;
    let _ = conn.close().await;

    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Legacy ping response sent to {}", client_addr
    );

    Ok(())
}

/// Handle a legacy login handshake (first byte was 0x02).
///
/// Reads the legacy handshake to extract hostname for routing, then connects to
/// the backend server and forwards all traffic bidirectionally (passthrough).
///
/// The backend server MUST support the legacy protocol — the proxy does not translate
/// between legacy and modern protocols.
pub async fn handle_legacy_login(
    mut conn: Connection,
    gateway: &Arc<Gateway>,
    session_id: Uuid,
    client_addr: SocketAddr,
) -> io::Result<()> {
    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Detected legacy login handshake from {}", client_addr
    );

    // Read and buffer the entire legacy handshake
    let raw_data = read_legacy_handshake_data(&mut conn).await?;

    // Parse it to extract hostname
    let handshake = parse_legacy_handshake(&raw_data).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse legacy handshake: {}", e),
        )
    })?;

    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Legacy handshake: proto={}, user={}, host={}, port={}",
        handshake.protocol_version,
        handshake.username,
        handshake.hostname,
        handshake.port
    );

    // Look up server config by hostname
    let server_config = if handshake.hostname.is_empty() {
        find_default_server(gateway).await
    } else {
        let found = gateway.find_server(&handshake.hostname).await;
        if found.is_none() {
            find_default_server(gateway).await
        } else {
            found
        }
    };

    let server_config = match server_config {
        Some(config) => config,
        None => {
            warn!(
                log_type = LogType::PacketProcessing.as_str(),
                "No server found for legacy login from {} (hostname: {})",
                client_addr,
                handshake.hostname
            );
            let _ = conn.close().await;
            return Ok(());
        }
    };

    // Connect to backend
    let server = Server::new(server_config).map_err(|e| {
        io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("Failed to create server: {}", e),
        )
    })?;

    let mut backend = match server.dial(session_id).await {
        Ok(conn) => conn,
        Err(e) => {
            warn!(
                log_type = LogType::PacketProcessing.as_str(),
                "Failed to connect to backend for legacy login: {}", e
            );
            let _ = conn.close().await;
            return Ok(());
        }
    };

    // Replay the buffered handshake bytes to the backend
    backend.write_raw(&raw_data).await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Failed to replay handshake to backend: {}", e),
        )
    })?;
    backend.flush().await.map_err(|e| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Failed to flush handshake to backend: {}", e),
        )
    })?;

    debug!(
        log_type = LogType::PacketProcessing.as_str(),
        "Legacy handshake replayed to backend, starting bidirectional forwarding"
    );

    // Reassemble TCP streams for zero-copy bidirectional forwarding
    let client_stream = conn.into_tcp_stream()?;
    let backend_stream = backend.into_tcp_stream()?;

    let (mut client_read, mut client_write) = client_stream.into_split();
    let (mut server_read, mut server_write) = backend_stream.into_split();

    let client_to_server = tokio::io::copy(&mut client_read, &mut server_write);
    let server_to_client = tokio::io::copy(&mut server_read, &mut client_write);

    tokio::select! {
        result = client_to_server => {
            if let Err(e) = result {
                debug!(log_type = LogType::TcpConnection.as_str(), "Legacy client->server copy ended: {}", e);
            }
        }
        result = server_to_client => {
            if let Err(e) = result {
                debug!(log_type = LogType::TcpConnection.as_str(), "Legacy server->client copy ended: {}", e);
            }
        }
    }

    Ok(())
}

/// Read all legacy ping data from the connection.
///
/// Legacy pings are small (at most ~100 bytes for 1.6 with MC|PingHost).
/// The 0xFE byte is still in the buffer (was peeked, not consumed).
async fn read_legacy_ping_data(conn: &mut Connection) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(256);

    // Read the 0xFE byte (still in buffer from peek)
    let mut fe_byte = [0u8; 1];
    conn.read_exact_raw(&mut fe_byte).await?;
    data.push(fe_byte[0]);

    // Try to read one more byte to distinguish Beta vs 1.4+/1.6
    let mut next = [0u8; 1];
    if let Ok(Ok(())) = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        conn.read_exact_raw(&mut next),
    )
    .await
    {
        data.push(next[0]);
        // If we got 0x01, try for more (0xFA + MC|PingHost for 1.6)
        if next[0] == 0x01
            && let Ok(Ok(more)) = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                read_remaining_v1_6_data(conn),
            )
            .await
        {
            data.extend_from_slice(&more);
        }
    }

    Ok(data)
}

/// Read the remaining bytes for a potential 1.6 MC|PingHost legacy ping.
async fn read_remaining_v1_6_data(conn: &mut Connection) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();

    // Try to read the 0xFA byte
    let mut byte = [0u8; 1];
    conn.read_exact_raw(&mut byte).await?;
    data.push(byte[0]);

    if byte[0] != 0xFA {
        return Ok(data); // Not 1.6 format
    }

    // Read channel name string length (short)
    let mut len_bytes = [0u8; 2];
    conn.read_exact_raw(&mut len_bytes).await?;
    data.extend_from_slice(&len_bytes);
    let str_len = u16::from_be_bytes(len_bytes) as usize;

    // Read channel name (UTF-16BE)
    let mut str_data = vec![0u8; str_len * 2];
    conn.read_exact_raw(&mut str_data).await?;
    data.extend_from_slice(&str_data);

    // Read data length (short)
    let mut data_len_bytes = [0u8; 2];
    conn.read_exact_raw(&mut data_len_bytes).await?;
    data.extend_from_slice(&data_len_bytes);
    let data_len = u16::from_be_bytes(data_len_bytes) as usize;

    // Read remaining data
    let mut remaining = vec![0u8; data_len];
    conn.read_exact_raw(&mut remaining).await?;
    data.extend_from_slice(&remaining);

    Ok(data)
}

/// Read all bytes of a legacy handshake (packet 0x02).
///
/// Supports both formats:
/// - **Pre-1.3**: `[0x02] [short: string_len] [UTF-16BE: "username;hostname:port"]`
/// - **1.3+**: `[0x02] [byte: proto] [short+UTF16: username] [short+UTF16: hostname] [i32: port]`
///
/// Detection: the second byte is `0x00` for pre-1.3 (high byte of string length)
/// or non-zero for 1.3+ (protocol version 39–78).
async fn read_legacy_handshake_data(conn: &mut Connection) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(256);

    // Read packet ID (0x02) — we know it's there from peek
    let mut packet_id = [0u8; 1];
    conn.read_exact_raw(&mut packet_id).await?;
    data.push(packet_id[0]);

    // Read the format detection byte
    let mut format_byte = [0u8; 1];
    conn.read_exact_raw(&mut format_byte).await?;
    data.push(format_byte[0]);

    if format_byte[0] == 0x00 {
        let mut low_byte = [0u8; 1];
        conn.read_exact_raw(&mut low_byte).await?;
        data.push(low_byte[0]);

        let str_len = u16::from_be_bytes([0x00, low_byte[0]]) as usize;

        // Read the UTF-16BE connection string
        let mut str_data = vec![0u8; str_len * 2];
        conn.read_exact_raw(&mut str_data).await?;
        data.extend_from_slice(&str_data);
    } else {

        let username_bytes = read_legacy_string_bytes(conn).await?;
        data.extend_from_slice(&username_bytes);

        let hostname_bytes = read_legacy_string_bytes(conn).await?;
        data.extend_from_slice(&hostname_bytes);

        let mut port = [0u8; 4];
        conn.read_exact_raw(&mut port).await?;
        data.extend_from_slice(&port);
    }

    Ok(data)
}

async fn read_legacy_string_bytes(conn: &mut Connection) -> io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 2];
    conn.read_exact_raw(&mut len_bytes).await?;
    let char_count = u16::from_be_bytes(len_bytes) as usize;

    let mut str_data = vec![0u8; char_count * 2];
    conn.read_exact_raw(&mut str_data).await?;

    let mut result = Vec::with_capacity(2 + str_data.len());
    result.extend_from_slice(&len_bytes);
    result.extend_from_slice(&str_data);
    Ok(result)
}

async fn find_default_server(gateway: &Gateway) -> Option<Arc<infrarust_config::ServerConfig>> {
    let configs = gateway
        .shared
        .configuration_service()
        .get_all_configurations()
        .await;

    configs.into_values().next()
}

async fn fetch_and_convert_to_legacy(
    variant: &LegacyPingVariant,
    server_config: &Arc<infrarust_config::ServerConfig>,
    session_id: Uuid,
) -> io::Result<Vec<u8>> {
    let server = Server::new(server_config.clone())
        .map_err(|e| io::Error::other(format!("Failed to create server: {}", e)))?;

    let hostname = variant.hostname().unwrap_or_else(|| {
        server_config
            .domains
            .first()
            .map(|s| s.as_str())
            .unwrap_or("localhost")
    });

    let request = build_synthetic_status_request(hostname, session_id)?;

    let status_packet = server
        .fetch_status_directly(&request)
        .await
        .map_err(|e| io::Error::other(format!("Backend status fetch failed: {}", e)))?;

    // Convert the modern JSON response to legacy kick format
    generate_legacy_motd_from_packet(&status_packet, variant)
}

fn build_synthetic_status_request(hostname: &str, session_id: Uuid) -> io::Result<ServerRequest> {
    let handshake = ServerBoundHandshake {
        protocol_version: VarInt(47), // 1.8 protocol (commonly supported)
        server_address: infrarust_protocol::types::ProtocolString(hostname.to_string()),
        server_port: infrarust_protocol::types::UnsignedShort(25565),
        next_state: VarInt(1), // STATUS
    };

    let mut handshake_packet = Packet::new(0x00);
    handshake_packet
        .encode(&handshake)
        .map_err(|e| io::Error::other(format!("Failed to encode synthetic handshake: {}", e)))?;

    let status_request_packet = Packet::new(SERVERBOUND_REQUEST_ID);

    Ok(ServerRequest {
        client_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        original_client_addr: None,
        domain: hostname.into(),
        is_login: false,
        protocol_version: Version::from(47),
        read_packets: Arc::new([handshake_packet, status_request_packet]),
        session_id,
    })
}

/// This mirrors the login handler's passthrough approach: send raw bytes, read raw response.
/// Works with any backend that handles the 0xFE legacy ping (both legacy and modern servers).
async fn forward_legacy_ping_to_backend(
    raw_ping_data: &[u8],
    server_config: &Arc<infrarust_config::ServerConfig>,
    session_id: Uuid,
) -> io::Result<Vec<u8>> {
    let server = Server::new(server_config.clone())
        .map_err(|e| io::Error::other(format!("Failed to create server: {}", e)))?;

    let mut backend = server
        .dial(session_id)
        .await
        .map_err(|e| io::Error::other(format!("Failed to connect to backend: {}", e)))?;

    backend
        .write_raw(raw_ping_data)
        .await
        .map_err(|e| io::Error::other(format!("Failed to send legacy ping to backend: {}", e)))?;
    backend
        .flush()
        .await
        .map_err(|e| io::Error::other(format!("Failed to flush legacy ping to backend: {}", e)))?;

    let response = read_legacy_kick_response(&mut backend).await?;

    let _ = backend.close().await;

    Ok(response)
}

/// Wire format: `[0xFF] [u16 BE: string_length_in_utf16_units] [UTF-16BE: payload]`
async fn read_legacy_kick_response(
    conn: &mut crate::network::connection::ServerConnection,
) -> io::Result<Vec<u8>> {
    let mut packet_id = [0u8; 1];
    conn.read_exact_raw(&mut packet_id).await?;

    if packet_id[0] != 0xFF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Expected legacy kick packet 0xFF, got 0x{:02X}",
                packet_id[0]
            ),
        ));
    }

    let mut len_bytes = [0u8; 2];
    conn.read_exact_raw(&mut len_bytes).await?;
    let str_len = u16::from_be_bytes(len_bytes) as usize;

    if str_len > 32767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Legacy kick string length too large: {}", str_len),
        ));
    }

    let byte_len = str_len * 2;
    let mut payload = vec![0u8; byte_len];
    conn.read_exact_raw(&mut payload).await?;

    let mut result = Vec::with_capacity(1 + 2 + byte_len);
    result.push(0xFF);
    result.extend_from_slice(&len_bytes);
    result.extend_from_slice(&payload);

    Ok(result)
}

fn generate_legacy_fallback(
    variant: &LegacyPingVariant,
    config: &infrarust_config::ServerConfig,
) -> Vec<u8> {
    let motd_config = config.motds.unreachable.as_ref();
    match generate_legacy_motd_for_state(&MotdState::Unreachable, motd_config, variant) {
        Ok(bytes) => bytes,
        Err(_) => {
            // Absolute fallback
            if variant.uses_v1_4_response_format() {
                build_legacy_kick_v1_4(0, "Infrarust", "Server unreachable", 0, 0)
            } else {
                build_legacy_kick_beta("Server unreachable", 0, 0)
            }
        }
    }
}

fn generate_legacy_no_server(variant: &LegacyPingVariant) -> Vec<u8> {
    if variant.uses_v1_4_response_format() {
        build_legacy_kick_v1_4(0, "Infrarust", "Unknown server", 0, 0)
    } else {
        build_legacy_kick_beta("Unknown server", 0, 0)
    }
}
