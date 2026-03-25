//! Shared helpers for connection handlers.

use std::sync::Arc;

use infrarust_api::types::{PlayerId, ServerId};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};
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
        .get_packet_id::<CLoginDisconnect>(ConnectionState::Login, Direction::Clientbound, version)
        .unwrap_or(0x00);

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
