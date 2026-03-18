//! Server switch orchestrator.
//!
//! Handles transferring a player from one backend server to another without
//! disconnecting them. The mechanism is version-dependent — see `switch_packets`
//! and `config_phase` submodules for details.

mod config_phase;
mod switch_packets;

use infrarust_api::event::ResultedEvent;
use infrarust_api::types::{GameProfile, PlayerId, ServerId};
use infrarust_protocol::packets::login::SLoginAcknowledged;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::pipeline::types::HandshakeData;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;

/// Successful server switch result.
pub struct SwitchSuccess {
    /// The new backend bridge (replaces the old one in the proxy loop).
    pub new_backend: BackendBridge,
    /// The server ID that was switched to.
    pub new_server_id: ServerId,
}

/// Performs a server switch: connects to a new backend, sends the appropriate
/// packets to the client, and returns the new backend bridge.
///
/// The caller should replace its `BackendBridge` with the returned one and
/// update the player session's `current_server`.
#[allow(clippy::too_many_arguments)]
pub async fn perform_switch(
    client: &mut ClientBridge,
    current_server: &ServerId,
    target: ServerId,
    handshake_data: &HandshakeData,
    game_profile_name: &str,
    player_id: PlayerId,
    api_profile: &GameProfile,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    peer_addr: std::net::SocketAddr,
    protocol_version: ProtocolVersion,
) -> Result<SwitchSuccess, CoreError> {
    let version = protocol_version;

    // 1. Resolve target server config
    let server_config = services
        .domain_router
        .find_by_server_id(target.as_str())
        .ok_or_else(|| {
            CoreError::Rejected(format!("unknown server: {}", target.as_str()))
        })?;

    // 2. Fire ServerPreConnectEvent (awaited — can deny/redirect)
    let pre_connect = infrarust_api::events::connection::ServerPreConnectEvent::new(
        player_id,
        api_profile.clone(),
        target.clone(),
    );
    let pre_connect = services.event_bus.fire(pre_connect).await;

    let effective_target = match pre_connect.result() {
        infrarust_api::events::connection::ServerPreConnectResult::Allowed => target.clone(),
        infrarust_api::events::connection::ServerPreConnectResult::ConnectTo(redirect) => {
            tracing::info!(
                original = %target,
                redirect = %redirect,
                "server switch redirected by event"
            );
            redirect.clone()
        }
        infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
            return Err(CoreError::Rejected(format!(
                "switch denied: {}",
                reason.to_json()
            )));
        }
        _ => target.clone(), // SendToLimbo, VirtualBackend — not implemented yet
    };

    // Re-resolve if redirected
    let server_config = if effective_target != target {
        services
            .domain_router
            .find_by_server_id(effective_target.as_str())
            .ok_or_else(|| {
                CoreError::Rejected(format!("unknown redirect server: {}", effective_target.as_str()))
            })?
    } else {
        server_config
    };

    // 3. Connect to new backend
    let connection_info = infrarust_transport::ConnectionInfo {
        peer_addr,
        real_ip: None,
        real_port: None,
        local_addr: peer_addr, // Not critical for outgoing backend connections
        connected_at: tokio::time::Instant::now(),
    };

    let backend_conn = backend_connector
        .connect(
            &effective_target.as_str(),
            &server_config.addresses,
            server_config.timeouts.as_ref().map(|t| t.connect),
            server_config.send_proxy_protocol,
            &connection_info,
        )
        .await
        .map_err(|e| CoreError::Rejected(format!("failed to connect to {}: {e}", effective_target.as_str())))?;

    let mut new_backend = BackendBridge::new(backend_conn.into_stream(), version);

    // 4. Send handshake + login start with offline UUID
    new_backend
        .send_initial_packets_offline(
            handshake_data,
            &server_config,
            game_profile_name,
            &services.packet_registry,
        )
        .await?;

    // 5. Consume backend login (SetCompression + LoginSuccess)
    new_backend
        .consume_backend_login(&services.packet_registry, version)
        .await?;

    // 6. For 1.20.2+: send LoginAcknowledged to backend, transition to Config
    if version.no_less_than(ProtocolVersion::V1_20_2) {
        let ack = SLoginAcknowledged;
        new_backend
            .send_packet(&ack, &services.packet_registry)
            .await?;
        new_backend.set_state(ConnectionState::Config);
        tracing::debug!("backend LoginAcknowledged → Config");
    }

    // 7. Fire ServerConnectedEvent (fire-and-forget)
    services.event_bus.fire_and_forget_arc(
        infrarust_api::events::connection::ServerConnectedEvent {
            player_id,
            server: effective_target.clone(),
        },
    );

    // 8. Version-branched switch
    let join_game_frame = if version.no_less_than(ProtocolVersion::V1_20_2) {
        // 1.20.2+: config phase → JoinGame
        config_phase::handle_config_phase_switch(
            client,
            &mut new_backend,
            &services.packet_registry,
            version,
        )
        .await?
    } else {
        // Pre-1.20.2: read JoinGame directly from new backend
        new_backend
            .read_frame()
            .await?
            .ok_or(CoreError::ConnectionClosed)?
    };

    // 9. Send switch packets to client (JoinGame + Respawn trick)
    switch_packets::send_switch_packets(
        client,
        &join_game_frame,
        version,
        &services.packet_registry,
    )
    .await?;

    // 10. Fire ServerSwitchEvent (fire-and-forget)
    services.event_bus.fire_and_forget_arc(
        infrarust_api::events::connection::ServerSwitchEvent {
            player_id,
            previous_server: current_server.clone(),
            new_server: effective_target.clone(),
        },
    );

    tracing::info!(
        previous = %current_server,
        new = %effective_target,
        "server switch complete"
    );

    Ok(SwitchSuccess {
        new_backend,
        new_server_id: effective_target,
    })
}
