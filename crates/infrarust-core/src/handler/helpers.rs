//! Shared helpers for connection handlers.

#[cfg(feature = "telemetry")]
use std::sync::Arc;

use infrarust_api::types::Component;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::packets::login::CLoginDisconnect;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_protocol::{Packet, PacketRegistry};

use crate::error::CoreError;
use crate::session::proxy_loop::ProxyLoopOutcome;

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
        ProxyLoopOutcome::Kicked { reason } => {
            tracing::info!(session = %session_id, reason = %reason.to_plain(), "player kicked");
        }
        ProxyLoopOutcome::BackendKick(kick) => {
            tracing::info!(session = %session_id, reason = %kick, "backend kicked the player");
        }
        ProxyLoopOutcome::BackendClosed { reason } => {
            tracing::info!(
                session = %session_id,
                reason = reason.as_ref().map(Component::to_plain),
                "backend dropped the player"
            );
        }
    }
}

/// Sends a login disconnect (kick) packet to a raw TCP stream.
pub(crate) async fn send_login_disconnect(
    stream: &mut tokio::net::TcpStream,
    reason: &Component,
    version: ProtocolVersion,
    packet_registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let packet = CLoginDisconnect {
        reason: crate::util::text::json_for(reason, version),
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
    use infrarust_api::types::{ClickEvent, NamedColor};
    use infrarust_protocol::{McBufReadExt, build_default_registry};
    use tokio::io::AsyncReadExt;

    use super::{Component, ProtocolVersion, send_login_disconnect};

    async fn kick_payload(reason: &Component, version: ProtocolVersion) -> String {
        let registry = build_default_registry();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();

        send_login_disconnect(&mut server, reason, version, &registry)
            .await
            .unwrap_or_else(|e| panic!("no kick for protocol {}: {e}", version.0));
        drop(server);

        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).await.unwrap();
        let mut frame = bytes.as_slice();
        let len = frame.read_var_int().unwrap().0 as usize;
        assert_eq!(len, frame.len());
        assert_eq!(frame.read_var_int().unwrap().0, 0x00);
        frame.read_string().unwrap()
    }

    #[tokio::test]
    async fn kick_reason_is_serialised_once_for_the_peer_version() {
        let reason = Component::text("No entry")
            .color(NamedColor::Red)
            .click(ClickEvent::OpenUrl("https://example.com".into()));
        let old: serde_json::Value =
            serde_json::from_str(&kick_payload(&reason, ProtocolVersion::V1_21_4).await).unwrap();
        assert_eq!(
            old,
            serde_json::json!({
                "text": "No entry",
                "color": "red",
                "clickEvent": {"action": "open_url", "value": "https://example.com"}
            })
        );
        let new: serde_json::Value =
            serde_json::from_str(&kick_payload(&reason, ProtocolVersion::V1_21_11).await).unwrap();
        assert_eq!(
            new,
            serde_json::json!({
                "text": "No entry",
                "color": "red",
                "click_event": {"action": "open_url", "url": "https://example.com"}
            })
        );
    }

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

            send_login_disconnect(&mut server, &Component::text("Banned"), version, &registry)
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
