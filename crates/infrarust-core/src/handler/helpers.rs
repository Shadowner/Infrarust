//! Shared helpers for connection handlers.

use std::sync::Arc;

use infrarust_api::types::{PlayerId, ServerId};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_protocol::{Packet, PacketRegistry};

use crate::error::CoreError;
use crate::event_bus::EventBusImpl;
use crate::session::proxy_loop::ProxyLoopOutcome;

/// Fires a `DisconnectEvent` on the event bus.
///
/// Called by all handlers at session teardown. Fire-and-return (not fire-and-forget)
/// because we want to ensure the event is processed before session cleanup.
pub(crate) async fn fire_disconnect_event(
    event_bus: &Arc<EventBusImpl>,
    player_id: PlayerId,
    username: String,
    last_server: Option<ServerId>,
) {
    let disconnect = infrarust_api::events::lifecycle::DisconnectEvent {
        player_id,
        username,
        last_server,
    };
    let _ = event_bus.fire(disconnect).await;
}

/// Logs the outcome of a proxy loop session with consistent formatting.
///
/// Used by offline and client_only handlers (passthrough uses its own
/// format with byte counters from the forwarder).
pub(crate) fn log_proxy_loop_outcome(session_id: &Uuid, outcome: &ProxyLoopOutcome) {
    match outcome {
        ProxyLoopOutcome::ClientDisconnected => {
            tracing::info!(session = %session_id, "client disconnected");
        }
        ProxyLoopOutcome::BackendDisconnected { reason } => {
            tracing::info!(session = %session_id, ?reason, "backend disconnected");
        }
        ProxyLoopOutcome::Shutdown => {
            tracing::debug!(session = %session_id, "shutdown");
        }
        ProxyLoopOutcome::Error(e) => {
            if e.is_expected_disconnect() {
                tracing::debug!(session = %session_id, error = %e, "session ended (expected)");
            } else {
                tracing::warn!(session = %session_id, error = %e, "session error");
            }
        }
        ProxyLoopOutcome::SwitchRequested { target } => {
            tracing::info!(session = %session_id, %target, "server switch requested");
        }
    }
}

/// Sends a login disconnect (kick) packet to a raw TCP stream.
pub(crate) async fn send_login_disconnect(
    stream: &mut tokio::net::TcpStream,
    reason: &str,
    version: ProtocolVersion,
    packet_registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let json_reason = serde_json::json!({"text": reason}).to_string();
    let packet = CLoginDisconnect {
        reason: json_reason,
    };

    let packet_id = packet_registry
        .get_packet_id::<CLoginDisconnect>(version)
        .or_else(|| packet_registry.get_packet_id::<CLoginDisconnect>(ProtocolVersion::V1_7_2))
        .ok_or_else(|| {
            CoreError::Protocol(infrarust_protocol::ProtocolError::invalid(format!(
                "no CLoginDisconnect id for protocol {}",
                version.0
            )))
        })?;

    let mut payload = Vec::new();
    packet.encode(&mut payload, version)?;

    let mut encoder = PacketEncoder::new();
    encoder.append_raw(packet_id, &payload)?;
    let bytes = encoder.take();

    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(feature = "telemetry")]
pub(crate) fn record_session_start(
    metrics: &Option<Arc<crate::telemetry::ProxyMetrics>>,
    config_id: &str,
    mode: &str,
) {
    if let Some(m) = metrics {
        m.record_connection_start(config_id, mode);
        m.record_player_join(config_id);
    }
}

#[cfg(feature = "telemetry")]
pub(crate) fn record_session_end(
    metrics: &Option<Arc<crate::telemetry::ProxyMetrics>>,
    duration: std::time::Duration,
    config_id: &str,
    mode: &str,
) {
    if let Some(m) = metrics {
        m.record_connection_end(duration.as_secs_f64(), config_id, mode);
        m.record_player_leave(config_id);
    }
}

#[cfg(test)]
mod tests {
    use infrarust_protocol::{McBufReadExt, build_default_registry};
    use tokio::io::AsyncReadExt;

    use super::{ProtocolVersion, send_login_disconnect};

    #[tokio::test]
    async fn kick_reaches_peers_below_the_first_mapping() {
        let registry = build_default_registry();

        for version in [
            ProtocolVersion(3),
            ProtocolVersion(0),
            ProtocolVersion(-1),
            ProtocolVersion::LEGACY,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (mut server, _) = listener.accept().await.unwrap();

            send_login_disconnect(&mut server, "Banned", version, &registry)
                .await
                .unwrap_or_else(|e| panic!("no kick for protocol {}: {e}", version.0));
            drop(server);

            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).await.unwrap();

            let mut frame = bytes.as_slice();
            let len = frame.read_var_int().unwrap().0 as usize;
            assert_eq!(len, frame.len());
            assert_eq!(frame.read_var_int().unwrap().0, 0x00);
            assert_eq!(frame.read_string().unwrap(), r#"{"text":"Banned"}"#);
        }
    }
}
