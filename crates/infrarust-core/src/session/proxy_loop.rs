//! Bidirectional packet forwarding loop between client and backend.
//!
//! This is the core of intercepted proxy modes. It reads packets from
//! both sides concurrently via `tokio::select!`, intercepts special
//! packets (SetCompression, LoginSuccess, Disconnect, FinishConfig),
//! and forwards everything else opaquely.

use tokio_util::sync::CancellationToken;

use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::config::{CFinishConfig, SAcknowledgeFinishConfig};
use infrarust_protocol::packets::login::{
    CLoginDisconnect, CLoginSuccess, CSetCompression, SLoginAcknowledged,
};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction};

use crate::error::CoreError;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;

/// Result of the proxy loop, determining what happens after the loop ends.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProxyLoopOutcome {
    /// Client closed its connection — full cleanup.
    ClientDisconnected,
    /// Backend closed its connection.
    /// In Phase 2A: cleanup. In Phase 4+: server switch / limbo.
    BackendDisconnected { reason: Option<String> },
    /// Global proxy shutdown.
    Shutdown,
    /// I/O or protocol error.
    Error(CoreError),
}

/// Action to take after processing a backend → client packet.
#[derive(Debug)]
#[non_exhaustive]
enum BackendAction {
    /// Continue the loop normally.
    Continue,
    /// Backend sent a disconnect packet.
    Disconnected(Option<String>),
}

/// Runs the bidirectional proxy loop between client and backend.
///
/// Both directions run concurrently via `tokio::select!`.
/// Special packets are intercepted for state management:
/// - `SetCompression`: activates compression on both bridges
/// - `LoginSuccess`: transitions Login → Config (1.20.2+) or Play
/// - `FinishConfig` / `AcknowledgeFinishConfig`: transitions Config → Play
/// - `Disconnect`: forwards and terminates
pub async fn proxy_loop(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    registry: &PacketRegistry,
    shutdown: CancellationToken,
) -> ProxyLoopOutcome {
    loop {
        tokio::select! {
            frame = client.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        if let Err(e) = handle_client_to_backend(client, backend, frame, registry).await {
                            return ProxyLoopOutcome::Error(e);
                        }
                    }
                    Ok(None) => return ProxyLoopOutcome::ClientDisconnected,
                    Err(e) => return ProxyLoopOutcome::Error(e),
                }
            }
            frame = backend.read_frame() => {
                match frame {
                    Ok(Some(frame)) => {
                        match handle_backend_to_client(client, backend, frame, registry).await {
                            Ok(BackendAction::Continue) => {}
                            Ok(BackendAction::Disconnected(reason)) => {
                                return ProxyLoopOutcome::BackendDisconnected { reason };
                            }
                            Err(e) => return ProxyLoopOutcome::Error(e),
                        }
                    }
                    Ok(None) => return ProxyLoopOutcome::BackendDisconnected { reason: None },
                    Err(e) => return ProxyLoopOutcome::Error(e),
                }
            }
            _ = shutdown.cancelled() => {
                return ProxyLoopOutcome::Shutdown;
            }
        }
    }
}

/// Handles a packet from the client, forwarding it to the backend.
///
/// In Phase 2A, most packets are forwarded opaquely.
/// `SLoginAcknowledged` and `SAcknowledgeFinishConfig` trigger state transitions.
async fn handle_client_to_backend(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    frame: PacketFrame,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let version = client.protocol_version;
    let state = client.state();

    // Try to decode for state transition detection
    match registry.decode_frame(&frame, state, Direction::Serverbound, version) {
        Ok(DecodedPacket::Typed { packet, .. }) => {
            if packet
                .as_any()
                .downcast_ref::<SLoginAcknowledged>()
                .is_some()
            {
                // Client acknowledged login success → transition to Config
                backend.write_frame(&frame).await?;
                client.set_state(ConnectionState::Config);
                backend.set_state(ConnectionState::Config);
                tracing::debug!("state transition: Login → Config (LoginAcknowledged)");
                return Ok(());
            }

            if packet
                .as_any()
                .downcast_ref::<SAcknowledgeFinishConfig>()
                .is_some()
            {
                // Client acknowledged finish config → transition to Play
                backend.write_frame(&frame).await?;
                client.set_state(ConnectionState::Play);
                backend.set_state(ConnectionState::Play);
                tracing::debug!("state transition: Config → Play (AcknowledgeFinishConfig)");
                return Ok(());
            }

            // All other typed packets: forward opaquely
            backend.write_frame(&frame).await?;
        }
        Ok(DecodedPacket::Opaque { .. }) | Err(_) => {
            // Unknown or decode error: forward opaquely
            backend.write_frame(&frame).await?;
        }
    }

    Ok(())
}

/// Handles a packet from the backend, forwarding it to the client.
///
/// Intercepts special packets for state management:
/// - `SetCompression`: activates compression on both bridges
/// - `LoginSuccess`: state transition Login → Config/Play
/// - `Disconnect`: forwards and signals loop termination
/// - `FinishConfig`: prepares Config → Play transition
async fn handle_backend_to_client(
    client: &mut ClientBridge,
    backend: &mut BackendBridge,
    frame: PacketFrame,
    registry: &PacketRegistry,
) -> Result<BackendAction, CoreError> {
    let version = client.protocol_version;
    let state = backend.state;

    match registry.decode_frame(&frame, state, Direction::Clientbound, version) {
        Ok(DecodedPacket::Typed { packet, .. }) => {
            // SetCompression — activate on both sides, forward to client
            if let Some(set_comp) = packet.as_any().downcast_ref::<CSetCompression>() {
                let threshold = set_comp.threshold.0;
                backend.set_compression(threshold);
                client.set_compression(threshold);
                client.write_frame(&frame).await?;
                tracing::debug!(threshold, "compression activated");
                return Ok(BackendAction::Continue);
            }

            // LoginSuccess — forward, transition state
            if packet.as_any().downcast_ref::<CLoginSuccess>().is_some() {
                client.write_frame(&frame).await?;
                // State transition happens when client sends LoginAcknowledged (1.20.2+)
                // or immediately for older versions
                if version.less_than(infrarust_protocol::version::ProtocolVersion::V1_20_2) {
                    client.set_state(ConnectionState::Play);
                    backend.set_state(ConnectionState::Play);
                    tracing::debug!("state transition: Login → Play (pre-1.20.2)");
                }
                // For 1.20.2+, transition happens in handle_client_to_backend
                // when SLoginAcknowledged is received
                return Ok(BackendAction::Continue);
            }

            // LoginDisconnect
            if let Some(disconnect) = packet.as_any().downcast_ref::<CLoginDisconnect>() {
                client.write_frame(&frame).await?;
                return Ok(BackendAction::Disconnected(Some(disconnect.reason.clone())));
            }

            // Play Disconnect
            if let Some(_disconnect) = packet.as_any().downcast_ref::<CDisconnect>() {
                client.write_frame(&frame).await?;
                return Ok(BackendAction::Disconnected(Some(
                    "backend disconnect".to_string(),
                )));
            }

            // FinishConfig — forward, state transition happens when client ACKs
            if packet.as_any().downcast_ref::<CFinishConfig>().is_some() {
                client.write_frame(&frame).await?;
                // Transition happens in handle_client_to_backend
                // when SAcknowledgeFinishConfig is received
                return Ok(BackendAction::Continue);
            }

            // All other typed packets: forward
            client.write_frame(&frame).await?;
        }
        Ok(DecodedPacket::Opaque { .. }) => {
            client.write_frame(&frame).await?;
        }
        Err(e) => {
            tracing::warn!("failed to decode backend frame: {e}");
            // Forward anyway (best effort)
            client.write_frame(&frame).await?;
        }
    }

    Ok(BackendAction::Continue)
}
