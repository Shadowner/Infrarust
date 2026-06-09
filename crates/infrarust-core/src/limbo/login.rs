//! Login for limbo — completes the config phase for direct limbo entry.
//!
//! Used when `ServerPreConnectResult::SendToLimbo` is returned at initial
//! connect in client_only mode. At that point, LoginSuccess + LoginAcknowledged
//! have already been handled by the auth flow, and the client is in Config state.
//! This module sends registry data (from cache or embedded) and transitions to Play.

use std::time::Duration;

use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig, SKnownPacks};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::session::client_bridge::ClientBridge;

const LIMBO_CONFIG_PHASE_TIMEOUT_SECS: u64 = 10;

/// Completes the configuration phase for a client entering limbo.
///
/// Uses the [`RegistryCodecCache`] which provides:
/// - captured frames (if a player already connected to a backend of this version)
/// - embedded data (if available for this version)
///
/// Works for both initial connect AND server switch to limbo.
///
/// # Precondition
///
/// The client is in `Config` state (LoginSuccess and LoginAcknowledged
/// have already been exchanged by the auth flow).
///
/// # Errors
///
/// Returns [`CoreError::Other`] if no registry data is available for the
/// client's protocol version, or [`CoreError::ConnectionClosed`] if the
/// client disconnects.
pub(crate) async fn complete_config_for_limbo(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
    codec_cache: &RegistryCodecCache,
) -> Result<(), CoreError> {
    // 1. KnownPacks handshake (>= 1.20.5, protocol >= 766)
    if let Ok(Some(kp_frame)) = codec_cache.get_known_packs_frame(version) {
        client.write_frame(&kp_frame).await?;

        // Wait for SKnownPacks from the client
        let skp_id = registry.get_packet_id::<SKnownPacks>(
            ConnectionState::Config,
            Direction::Serverbound,
            version,
        );

        absorb_until(
            client,
            skp_id,
            "limbo login: client did not send KnownPacks in time",
        )
        .await?;
    }

    // 2. Send CRegistryData frames
    let frames = codec_cache.get_registry_frames(version)?;
    for frame in &frames {
        client.write_frame(frame).await?;
    }

    // 3. Send CFinishConfig
    let finish_id = registry
        .get_packet_id::<CFinishConfig>(ConnectionState::Config, Direction::Clientbound, version)
        .ok_or_else(|| {
            CoreError::Other(format!(
                "no packet ID for CFinishConfig in Config/Clientbound/{version:?}"
            ))
        })?;
    let mut finish_payload = Vec::new();
    CFinishConfig
        .encode(&mut finish_payload, version)
        .map_err(|e| CoreError::Other(e.to_string()))?;
    let finish_frame = infrarust_protocol::io::PacketFrame {
        id: finish_id,
        payload: bytes::Bytes::from(finish_payload),
    };
    client.write_frame(&finish_frame).await?;

    // 4. Wait for SAcknowledgeFinishConfig, absorbing any other client packets
    let ack_id = registry.get_packet_id::<SAcknowledgeFinishConfig>(
        ConnectionState::Config,
        Direction::Serverbound,
        version,
    );

    absorb_until(
        client,
        ack_id,
        "limbo login: client did not acknowledge finish config in time",
    )
    .await?;

    // 5. Transition to Play
    client.set_state(ConnectionState::Play);

    Ok(())
}

/// Reads (and discards) client config-phase packets until one with `target_id`
/// arrives, bounded by [`LIMBO_CONFIG_PHASE_TIMEOUT_SECS`]. The timeout wraps the
/// whole loop (not each read), so a client that drip-feeds unrelated packets
/// cannot keep the connection alive indefinitely.
async fn absorb_until(
    client: &mut ClientBridge,
    target_id: Option<i32>,
    timeout_message: &'static str,
) -> Result<(), CoreError> {
    tokio::time::timeout(
        Duration::from_secs(LIMBO_CONFIG_PHASE_TIMEOUT_SECS),
        async {
            loop {
                let frame = client
                    .read_frame()
                    .await?
                    .ok_or(CoreError::ConnectionClosed)?;

                if Some(frame.id) == target_id {
                    break;
                }
                tracing::trace!(
                    id = frame.id,
                    "absorbing client config packet during limbo login"
                );
            }
            Ok::<(), CoreError>(())
        },
    )
    .await
    .map_err(|_| CoreError::Timeout(timeout_message.into()))?
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn absorb_until_times_out_when_client_is_silent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _peer = TcpStream::connect(addr).await.unwrap(); // stays silent
        let (server_side, _) = listener.accept().await.unwrap();

        let mut client = ClientBridge::new(server_side, BytesMut::new(), ProtocolVersion::V1_21);

        let result = absorb_until(&mut client, Some(0x42), "test timeout").await;
        assert!(
            matches!(result, Err(CoreError::Timeout(_))),
            "expected a timeout, got {result:?}"
        );
    }
}
