//! Login for limbo — completes the config phase for direct limbo entry.
//!
//! Used when `ServerPreConnectResult::SendToLimbo` is returned at initial
//! connect in client_only mode. At that point, LoginSuccess + LoginAcknowledged
//! have already been handled by the auth flow, and the client is in Config state.
//! This module sends registry data (from cache or embedded) and transitions to Play.

use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig, SKnownPacks};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::session::client_bridge::ClientBridge;

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

        loop {
            let frame = client
                .read_frame()
                .await?
                .ok_or(CoreError::ConnectionClosed)?;

            if Some(frame.id) == skp_id {
                break;
            }
            // Absorb other client config packets (brand, settings, etc.)
            tracing::trace!(
                id = frame.id,
                "absorbing client config packet during limbo login (known packs phase)"
            );
        }
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

    loop {
        let frame = client
            .read_frame()
            .await?
            .ok_or(CoreError::ConnectionClosed)?;

        if Some(frame.id) == ack_id {
            tracing::debug!("client acknowledged finish config (limbo login)");
            break;
        }

        // Absorb all other client config packets
        tracing::trace!(
            id = frame.id,
            "absorbing client config packet during limbo login"
        );
    }

    // 5. Transition to Play
    client.set_state(ConnectionState::Play);

    Ok(())
}
