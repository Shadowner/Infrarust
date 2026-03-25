use std::time::{Duration, Instant};

use infrarust_api::services::config_service::ServerConfig;
use infrarust_protocol::codec::VarInt;
use infrarust_protocol::io::{PacketDecoder, PacketEncoder};
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::handshake::SHandshake;
use infrarust_protocol::packets::status::{
    CPingResponse, CStatusResponse, SPingRequest, SStatusRequest,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::dto::server::{HealthCheckResponse, PlayerSampleResponse};
use crate::util::now_iso8601;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V1_21;
const PROTOCOL_ID: i32 = 767;

/// Lightweight server health checker using the Minecraft status protocol.
pub struct HealthChecker;

impl Default for HealthChecker {
    fn default() -> Self {
        Self
    }
}

impl HealthChecker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn check(&self, config: &ServerConfig) -> HealthCheckResponse {
        let addr = match config.addresses.first() {
            Some(a) => a,
            None => return HealthCheckResponse::error("No address configured"),
        };

        let target = format!("{}:{}", addr.host, addr.port);
        let domain = config
            .domains
            .first()
            .map(|d| d.as_str())
            .unwrap_or(&addr.host);

        match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, self.ping(&target, domain, addr.port))
            .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => HealthCheckResponse::error(&format!("Health check failed: {e}")),
            Err(_) => HealthCheckResponse::error("Health check timed out (5s)"),
        }
    }

    async fn ping(
        &self,
        target: &str,
        domain: &str,
        port: u16,
    ) -> Result<HealthCheckResponse, String> {
        let mut stream = TcpStream::connect(target)
            .await
            .map_err(|e| format!("Connection refused: {e}"))?;

        // Send Handshake (intent=Status)
        let handshake = SHandshake {
            protocol_version: VarInt(PROTOCOL_ID),
            server_address: domain.to_string(),
            server_port: port,
            next_state: ConnectionState::Status,
        };
        self.send_packet(&mut stream, 0x00, &handshake).await?;

        // Send StatusRequest
        self.send_packet(&mut stream, 0x00, &SStatusRequest).await?;

        // Read StatusResponse
        let mut decoder = PacketDecoder::new();
        let status_frame = self.read_frame(&mut stream, &mut decoder, 0x00).await?;
        let status_response =
            CStatusResponse::decode(&mut status_frame.as_slice(), PROTOCOL_VERSION)
                .map_err(|e| format!("Failed to decode status response: {e}"))?;

        // Send PingRequest + read PingResponse for latency
        let ping_start = Instant::now();
        let ping_payload = ping_start.elapsed().as_millis() as i64;
        self.send_packet(
            &mut stream,
            0x01,
            &SPingRequest {
                payload: ping_payload,
            },
        )
        .await?;

        let pong_frame = self.read_frame(&mut stream, &mut decoder, 0x01).await?;
        let _pong = CPingResponse::decode(&mut pong_frame.as_slice(), PROTOCOL_VERSION)
            .map_err(|e| format!("Failed to decode pong: {e}"))?;
        let latency = ping_start.elapsed();

        // Parse the JSON response
        let parsed: ServerPingJson = serde_json::from_str(&status_response.json_response)
            .map_err(|e| format!("Invalid status JSON: {e}"))?;

        Ok(HealthCheckResponse {
            online: true,
            latency_ms: Some(latency.as_millis() as u64),
            motd_plain: Some(strip_motd_formatting(&parsed.description)),
            motd: Some(parsed.description),
            version_name: Some(parsed.version.name),
            version_protocol: Some(parsed.version.protocol),
            players_online: Some(parsed.players.online),
            players_max: Some(parsed.players.max),
            player_sample: parsed
                .players
                .sample
                .unwrap_or_default()
                .into_iter()
                .map(|s| PlayerSampleResponse {
                    name: s.name,
                    id: s.id,
                })
                .collect(),
            favicon: parsed.favicon,
            error: None,
            checked_at: now_iso8601(),
        })
    }

    async fn send_packet<P: Packet>(
        &self,
        stream: &mut TcpStream,
        packet_id: i32,
        packet: &P,
    ) -> Result<(), String> {
        let mut payload = Vec::new();
        packet
            .encode(&mut payload, PROTOCOL_VERSION)
            .map_err(|e| format!("Failed to encode packet: {e}"))?;

        let mut encoder = PacketEncoder::new();
        encoder
            .append_raw(packet_id, &payload)
            .map_err(|e| format!("Failed to frame packet: {e}"))?;

        let bytes = encoder.take();
        stream
            .write_all(&bytes)
            .await
            .map_err(|e| format!("Failed to write: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| format!("Failed to flush: {e}"))?;
        Ok(())
    }

    async fn read_frame(
        &self,
        stream: &mut TcpStream,
        decoder: &mut PacketDecoder,
        expected_id: i32,
    ) -> Result<Vec<u8>, String> {
        let mut buf = [0u8; 8192];
        loop {
            if let Some(frame) = decoder
                .try_next_frame()
                .map_err(|e| format!("Frame decode error: {e}"))?
            {
                if frame.id == expected_id {
                    return Ok(frame.payload.to_vec());
                }
                // Skip unexpected packets
                continue;
            }

            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| format!("Read error: {e}"))?;
            if n == 0 {
                return Err("Connection closed unexpectedly".to_string());
            }
            decoder.queue_bytes(&buf[..n]);
        }
    }
}

// Minimal JSON structures for parsing the status response
#[derive(Deserialize)]
struct ServerPingJson {
    version: PingVersionJson,
    players: PingPlayersJson,
    description: serde_json::Value,
    favicon: Option<String>,
}

#[derive(Deserialize)]
struct PingVersionJson {
    name: String,
    protocol: i32,
}

#[derive(Deserialize)]
struct PingPlayersJson {
    max: i32,
    online: i32,
    sample: Option<Vec<PingPlayerSampleJson>>,
}

#[derive(Deserialize)]
struct PingPlayerSampleJson {
    name: String,
    id: String,
}

/// Extracts plain text from a Minecraft MOTD, stripping `§X` formatting codes.
/// Handles both plain strings and JSON chat component objects.
fn strip_motd_formatting(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(obj) => {
            let mut out = String::new();
            if let Some(serde_json::Value::String(text)) = obj.get("text") {
                out.push_str(text);
            }
            if let Some(serde_json::Value::Array(extra)) = obj.get("extra") {
                for child in extra {
                    out.push_str(&strip_motd_formatting(child));
                }
            }
            return out;
        }
        serde_json::Value::Array(arr) => {
            return arr.iter().map(strip_motd_formatting).collect();
        }
        _ => return String::new(),
    };

    // Strip §X formatting codes (§ followed by any character)
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            chars.next(); // skip the formatting code character
        } else {
            result.push(ch);
        }
    }
    result
}
