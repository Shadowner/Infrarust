//! Configuration phase handling for server switch (1.20.2+).
//!
//! When switching servers on 1.20.2+, the proxy tells the client to re-enter
//! the configuration phase, forwards config packets from the new backend
//! opaquely, then transitions back to Play.

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::play::start_configuration::{
    CStartConfiguration, SAcknowledgeConfiguration,
};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;

/// Handles the configuration phase during a server switch for 1.20.2+.
///
/// Flow:
/// 1. Send `CStartConfiguration` to client (Play → Config transition)
/// 2. Wait for client's `SAcknowledgeConfiguration`
/// 3. Forward config packets from backend to client opaquely
/// 4. When backend sends `CFinishConfig`, forward to client
/// 5. Wait for client's `SAcknowledgeFinishConfig`, forward to backend
/// 6. Transition both sides back to Play
///
/// Returns the JoinGame frame read from the backend after config phase completes.
pub async fn handle_config_phase_switch(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    registry: &PacketRegistry,
    version: ProtocolVersion,
) -> Result<PacketFrame, CoreError> {
    // 1. Send StartConfiguration to client
    client
        .send_packet(&CStartConfiguration, registry)
        .await?;
    tracing::debug!("sent StartConfiguration to client");

    // 2. Read from client until we get AcknowledgeConfiguration
    loop {
        let frame = client
            .read_frame()
            .await?
            .ok_or(CoreError::ConnectionClosed)?;

        let ack_id = registry.get_packet_id::<SAcknowledgeConfiguration>(
            ConnectionState::Play,
            Direction::Serverbound,
            version,
        );

        if Some(frame.id) == ack_id {
            tracing::debug!("client acknowledged configuration");
            break;
        }
        // Absorb/ignore other Play-state packets during transition
        tracing::trace!(id = frame.id, "absorbing client packet during config transition");
    }

    // 3. Transition both sides to Config
    client.set_state(ConnectionState::Config);
    backend.set_state(ConnectionState::Config);

    // 4. Forward config packets bidirectionally until FinishConfig
    let finish_config_id = registry.get_packet_id::<CFinishConfig>(
        ConnectionState::Config,
        Direction::Clientbound,
        version,
    );

    let ack_finish_id = registry.get_packet_id::<SAcknowledgeFinishConfig>(
        ConnectionState::Config,
        Direction::Serverbound,
        version,
    );

    // Forward backend config packets to client
    loop {
        // Use select to handle packets from both sides during config negotiation
        tokio::select! {
            frame = backend.read_frame() => {
                let frame = frame?.ok_or(CoreError::ConnectionClosed)?;

                if Some(frame.id) == finish_config_id {
                    // Forward FinishConfig to client
                    client.write_frame(&frame).await?;
                    tracing::debug!("forwarded FinishConfig to client");
                    break;
                }

                // Forward all other config packets opaquely (RegistryData, KnownPacks, Tags, etc.)
                client.write_frame(&frame).await?;
            }
            frame = client.read_frame() => {
                let frame = frame?.ok_or(CoreError::ConnectionClosed)?;
                // Forward client responses to backend (e.g., SKnownPacks)
                backend.write_frame(&frame).await?;
            }
        }
    }

    // 5. Wait for client's AcknowledgeFinishConfig
    loop {
        let frame = client
            .read_frame()
            .await?
            .ok_or(CoreError::ConnectionClosed)?;

        if Some(frame.id) == ack_finish_id {
            // Forward the ack to the backend
            backend.write_frame(&frame).await?;
            tracing::debug!("client acknowledged finish config");
            break;
        }
        // Absorb any other packets
        tracing::trace!(id = frame.id, "absorbing client packet waiting for finish ack");
    }

    // 6. Transition both sides back to Play
    client.set_state(ConnectionState::Play);
    backend.set_state(ConnectionState::Play);

    // 7. Read JoinGame from backend (first Play-state packet)
    let join_game_frame = backend
        .read_frame()
        .await?
        .ok_or(CoreError::ConnectionClosed)?;

    tracing::debug!(id = join_game_frame.id, "received JoinGame from new backend");

    Ok(join_game_frame)
}
