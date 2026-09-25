//! Server switch orchestrator.
//!
//! Handles transferring a player from one backend server to another without
//! disconnecting them. The mechanism is version-dependent — see `switch_packets`
//! and `config_phase` submodules for details.

mod config_phase;
mod switch_packets;
mod validation;

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{ConnectCause, ServerPreConnectResult};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::login::SLoginAcknowledged;
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::forwarding::{ForwardingData, build_handshake_for_backend};
use crate::pipeline::types::HandshakeData;
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;
use crate::session::kick::{BackendKick, Kick};
use crate::session::server_join::{ServerJoin, pre_connect};

use config_phase::PhaseError;

const SWITCH_CONFIG_PHASE_TIMEOUT_SECS: u64 = 30;

/// Successful server switch result.
pub struct SwitchSuccess {
    /// The new backend bridge (replaces the old one in the proxy loop).
    pub new_backend: BackendBridge,
    /// The server ID that was switched to.
    pub new_server_id: ServerId,
}

pub(crate) enum SwitchResult {
    Backend(SwitchSuccess),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Denied(Component),
    Unchanged,
    Failed(Kick),
}

pub(crate) enum SwitchTarget {
    Unapproved {
        server: ServerId,
        cause: ConnectCause,
    },
    Approved(ServerId),
}

impl SwitchTarget {
    const fn server(&self) -> &ServerId {
        match self {
            Self::Unapproved { server, .. } | Self::Approved(server) => server,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn perform_switch(
    client: &mut ClientBridge,
    current_server: &ServerId,
    target: SwitchTarget,
    handshake_data: &HandshakeData,
    game_profile_name: &str,
    session: &Arc<PlayerSession>,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    protocol_version: ProtocolVersion,
) -> Result<SwitchResult, CoreError> {
    let version = protocol_version;
    let api_profile = session.game_profile();
    let requested = target.server().clone();

    let (server_config, load_balancer) = services
        .domain_router
        .find_route_by_server_id(requested.as_str())
        .ok_or_else(|| CoreError::Rejected(format!("unknown server: {}", requested.as_str())))?;

    let current_config = services
        .domain_router
        .find_by_server_id(current_server.as_str())
        .ok_or_else(|| {
            CoreError::Rejected(format!(
                "unknown current server: {}",
                current_server.as_str()
            ))
        })?;

    validation::validate_switch_allowed(&current_config, &server_config)
        .map_err(|e| CoreError::Rejected(e.to_string()))?;

    let effective_target = match target {
        SwitchTarget::Approved(server) => server,
        SwitchTarget::Unapproved { server, cause } => {
            let pre_connect =
                pre_connect(&services.event_bus, session, server.clone(), cause).await;
            let effective = match pre_connect.result() {
                ServerPreConnectResult::Allowed => server,
                ServerPreConnectResult::ConnectTo(redirect) => {
                    tracing::info!(
                        original = %server,
                        redirect = %redirect,
                        "server switch redirected by event"
                    );
                    redirect.clone()
                }
                ServerPreConnectResult::Denied { reason } => {
                    return Ok(SwitchResult::Denied(reason.clone()));
                }
                ServerPreConnectResult::SendToLimbo { limbo_handlers } => {
                    tracing::info!("server switch redirected to limbo by event");
                    let handler_names = if limbo_handlers.is_empty() {
                        server_config.limbo_handlers.clone()
                    } else {
                        limbo_handlers.clone()
                    };
                    let handlers = services
                        .limbo_handler_registry
                        .resolve_handlers_lenient(&handler_names);
                    let ctx = LimboEntryContext::PluginRedirect {
                        from_server: Some(current_server.clone()),
                    };
                    return Ok(SwitchResult::Limbo(handlers, ctx));
                }
                _ => server,
            };
            if cause == ConnectCause::Switch && effective == *current_server {
                return Ok(SwitchResult::Unchanged);
            }
            effective
        }
    };

    let (server_config, load_balancer) = if effective_target == requested {
        (server_config, load_balancer)
    } else {
        services
            .domain_router
            .find_route_by_server_id(effective_target.as_str())
            .ok_or_else(|| {
                CoreError::Rejected(format!(
                    "unknown redirect server: {}",
                    effective_target.as_str()
                ))
            })?
    };

    let connection_info = infrarust_transport::ConnectionInfo {
        peer_addr,
        real_ip,
        real_port: None,
        local_addr: peer_addr, // Not critical for outgoing backend connections
        connected_at: tokio::time::Instant::now(),
    };

    // Same strategy + unhealthy-last ordering as the login pipeline.
    let addresses = crate::loadbalancer::select_backend_addresses(
        &server_config,
        load_balancer.as_ref(),
        services.pending_backends.as_ref(),
        services.backend_health.as_ref(),
    );

    let backend_conn = match backend_connector
        .connect(
            effective_target.as_str(),
            &addresses,
            server_config.timeouts.as_ref().map(|t| t.connect),
            server_config.send_proxy_protocol,
            &connection_info,
        )
        .await
    {
        Ok(conn) => conn,
        Err(e) => {
            return Ok(SwitchResult::Failed(Kick::failed(
                effective_target,
                e.into(),
                false,
            )));
        }
    };

    let connected_address = backend_conn.server_address().clone();
    let mut new_backend = BackendBridge::new(backend_conn.into_stream(), version)
        .with_server_address(connected_address);

    if let Err(e) = login_to_backend(
        &mut new_backend,
        handshake_data,
        game_profile_name,
        api_profile,
        &server_config,
        services,
        peer_addr,
        real_ip,
        version,
    )
    .await
    {
        return Ok(SwitchResult::Failed(Kick::failed(
            effective_target,
            e,
            false,
        )));
    }

    let mut join = ServerJoin::new(session, effective_target.clone());
    join.connected(&services.event_bus).await;

    let mut stranded = client.state() != ConnectionState::Play;
    let join_game_frame = if version.no_less_than(ProtocolVersion::V1_20_2) {
        let session_token = session.shutdown_token().clone();
        let config_phase = tokio::time::timeout(
            std::time::Duration::from_secs(SWITCH_CONFIG_PHASE_TIMEOUT_SECS),
            config_phase::handle_config_phase_switch(
                client,
                &mut new_backend,
                &services.packet_registry,
                version,
                &mut stranded,
            ),
        );
        let phase = tokio::select! {
            () = session_token.cancelled() => return Err(CoreError::ConnectionClosed),
            phase = config_phase => phase,
        };
        match phase {
            Ok(Ok(frame)) => frame,
            Ok(Err(PhaseError::Client(e))) => return Err(e),
            Ok(Err(PhaseError::Backend(e))) => {
                return Ok(SwitchResult::Failed(Kick::failed(
                    effective_target,
                    e,
                    stranded,
                )));
            }
            Err(_) => {
                return Ok(SwitchResult::Failed(Kick::failed(
                    effective_target,
                    CoreError::Timeout("server switch config phase timed out".into()),
                    stranded,
                )));
            }
        }
    } else {
        let read = new_backend.read_frame().await;
        match read {
            Ok(Some(frame))
                if services
                    .packet_registry
                    .get_packet_id::<CDisconnect>(version)
                    == Some(frame.id) =>
            {
                let kick = BackendKick::new(frame, ConnectionState::Play, version);
                return Ok(SwitchResult::Failed(Kick::failed(
                    effective_target,
                    CoreError::BackendKick(Box::new(kick)),
                    stranded,
                )));
            }
            Ok(Some(frame)) => frame,
            Ok(None) => {
                return Ok(SwitchResult::Failed(Kick::failed(
                    effective_target,
                    CoreError::ConnectionClosed,
                    stranded,
                )));
            }
            Err(e) => {
                return Ok(SwitchResult::Failed(Kick::failed(
                    effective_target,
                    e,
                    stranded,
                )));
            }
        }
    };

    switch_packets::send_switch_packets(
        client,
        &join_game_frame,
        version,
        &services.packet_registry,
    )
    .await?;

    join.joined(&services.event_bus).await;

    tracing::info!(
        previous = %current_server,
        new = %effective_target,
        "server switch complete"
    );

    Ok(SwitchResult::Backend(SwitchSuccess {
        new_backend,
        new_server_id: effective_target,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn login_to_backend(
    new_backend: &mut BackendBridge,
    handshake_data: &HandshakeData,
    game_profile_name: &str,
    profile: &infrarust_api::types::GameProfile,
    server_config: &infrarust_config::ServerConfig,
    services: &ProxyServices,
    peer_addr: std::net::SocketAddr,
    real_ip: Option<std::net::IpAddr>,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let handler = services.resolve_forwarding_handler(server_config);
    let fwd_data = ForwardingData {
        real_ip: real_ip.unwrap_or(peer_addr.ip()),
        uuid: profile.uuid,
        username: game_profile_name.to_string(),
        properties: profile.properties.clone(),
        protocol_version: version,
        chat_session: None,
    };

    if handler.modifies_handshake() {
        let mut hs = build_handshake_for_backend(handshake_data, server_config);
        handler.apply_handshake(&mut hs, &fwd_data);
        new_backend
            .send_handshake_and_login(&hs, game_profile_name, &services.packet_registry)
            .await?;
    } else {
        new_backend
            .send_initial_packets_offline(
                handshake_data,
                server_config,
                game_profile_name,
                &services.packet_registry,
            )
            .await?;
    }

    let velocity_ctx = services.forwarding_secret().map(|s| (&fwd_data, s));
    new_backend
        .consume_backend_login(&services.packet_registry, version, velocity_ctx)
        .await?;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        new_backend
            .send_packet(&SLoginAcknowledged, &services.packet_registry)
            .await?;
        new_backend.set_state(ConnectionState::Config);
        tracing::debug!("backend LoginAcknowledged → Config");
    }
    Ok(())
}
