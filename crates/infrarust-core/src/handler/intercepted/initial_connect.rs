//! Initial server resolution and backend connection for intercepted modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_protocol::packets::login::SLoginAcknowledged;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use infrarust_transport::BackendConnector;

use super::auth::AuthResult;
use crate::error::CoreError;
use crate::forwarding::{ForwardingData, build_handshake_for_backend};
use crate::limbo::registry::LimboHandlerRegistry;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;
use crate::session::client_bridge::ClientBridge;

pub(super) enum ConnectionMode {
    Backend(BackendBridge),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
}

pub(super) enum InitialMode {
    Connected {
        mode: Box<ConnectionMode>,
        server_id: infrarust_api::types::ServerId,
    },
    /// Disconnect already sent.
    Denied,
}

fn resolve_limbo_strict(
    registry: &LimboHandlerRegistry,
    names: &[String],
) -> Option<Vec<Arc<dyn LimboHandler>>> {
    match registry.resolve_handlers(names) {
        Ok(h) if !h.is_empty() => Some(h),
        _ => None,
    }
}

fn resolve_limbo_lenient(
    registry: &LimboHandlerRegistry,
    names: &[String],
) -> Option<Vec<Arc<dyn LimboHandler>>> {
    let handlers = registry.resolve_handlers_lenient(names);
    if handlers.is_empty() {
        None
    } else {
        Some(handlers)
    }
}

async fn deny_no_limbo_handlers(
    client: &mut ClientBridge,
    services: &ProxyServices,
) -> Result<InitialMode, CoreError> {
    tracing::warn!("SendToLimbo at initial connect but no handlers resolved");
    client
        .disconnect("No limbo handlers configured", &services.packet_registry)
        .await
        .ok();
    Ok(InitialMode::Denied)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_initial_mode(
    client: &mut ClientBridge,
    auth_result: &AuthResult,
    login_completed: &mut bool,
    routing: &RoutingData,
    handshake: &HandshakeData,
    version: ProtocolVersion,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    connection_info: &infrarust_transport::ConnectionInfo,
) -> Result<InitialMode, CoreError> {
    let server_config = &routing.server_config;
    let player_id = auth_result.player_id;
    let api_profile = &auth_result.api_profile;

    let initial_server = infrarust_api::types::ServerId::new(routing.config_id.clone());
    let choose = infrarust_api::events::connection::PlayerChooseInitialServerEvent::new(
        player_id,
        api_profile.clone(),
        initial_server.clone(),
    );
    let choose = services.event_bus.fire(choose).await;

    let mut initial_mode: Option<ConnectionMode> = None;
    let target_server_id = match choose.result() {
        infrarust_api::events::connection::PlayerChooseInitialServerResult::Allowed => {
            initial_server.clone()
        }
        infrarust_api::events::connection::PlayerChooseInitialServerResult::Redirect(id) => {
            id.clone()
        }
        infrarust_api::events::connection::PlayerChooseInitialServerResult::SendToLimbo {
            limbo_handlers,
        } => {
            prepare_client_for_limbo(client, auth_result, login_completed, version, services)
                .await?;
            let Some(handlers) =
                resolve_limbo_strict(&services.limbo_handler_registry, limbo_handlers)
            else {
                return deny_no_limbo_handlers(client, services).await;
            };
            initial_mode = Some(ConnectionMode::Limbo(
                handlers,
                LimboEntryContext::InitialConnection {
                    target_server: initial_server.clone(),
                },
            ));
            initial_server.clone()
        }
        _ => initial_server.clone(),
    };

    if initial_mode.is_none() {
        let pre_connect = infrarust_api::events::connection::ServerPreConnectEvent::new(
            player_id,
            api_profile.clone(),
            target_server_id.clone(),
        );
        let pre_connect = services.event_bus.fire(pre_connect).await;
        match pre_connect.result() {
            infrarust_api::events::connection::ServerPreConnectResult::Allowed => {}
            infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
                let reason_json = reason.to_json();
                client
                    .disconnect(&reason_json, &services.packet_registry)
                    .await
                    .ok();
                return Ok(InitialMode::Denied);
            }
            infrarust_api::events::connection::ServerPreConnectResult::SendToLimbo {
                limbo_handlers,
            } => {
                prepare_client_for_limbo(client, auth_result, login_completed, version, services)
                    .await?;
                let handler_names = if limbo_handlers.is_empty() {
                    server_config.limbo_handlers.clone()
                } else {
                    limbo_handlers.clone()
                };
                let Some(handlers) =
                    resolve_limbo_lenient(&services.limbo_handler_registry, &handler_names)
                else {
                    return deny_no_limbo_handlers(client, services).await;
                };
                initial_mode = Some(ConnectionMode::Limbo(
                    handlers,
                    LimboEntryContext::InitialConnection {
                        target_server: initial_server.clone(),
                    },
                ));
            }
            _ => {} // ConnectTo, VirtualBackend -- Phase 4
        }
    }

    if initial_mode.is_none() && !server_config.limbo_handlers.is_empty() {
        prepare_client_for_limbo(client, auth_result, login_completed, version, services).await?;
        if let Some(handlers) = resolve_limbo_lenient(
            &services.limbo_handler_registry,
            &server_config.limbo_handlers,
        ) {
            initial_mode = Some(ConnectionMode::Limbo(
                handlers,
                LimboEntryContext::InitialConnection {
                    target_server: initial_server.clone(),
                },
            ));
        }
    }

    let mode = if let Some(limbo_mode) = initial_mode {
        limbo_mode
    } else {
        match connect_to_backend(
            client,
            auth_result,
            *login_completed,
            routing,
            handshake,
            version,
            services,
            backend_connector,
            connection_info,
        )
        .await
        {
            Ok(backend) => ConnectionMode::Backend(backend),
            Err(e) => {
                if !server_config.limbo_handlers.is_empty() {
                    tracing::info!(
                        server = %routing.config_id,
                        error = %e,
                        "backend unreachable, falling back to limbo"
                    );

                    prepare_client_for_limbo(
                        client,
                        auth_result,
                        login_completed,
                        version,
                        services,
                    )
                    .await?;
                    if let Some(handlers) = resolve_limbo_lenient(
                        &services.limbo_handler_registry,
                        &server_config.limbo_handlers,
                    ) {
                        ConnectionMode::Limbo(
                            handlers,
                            LimboEntryContext::KickedFromServer {
                                server: target_server_id.clone(),
                                reason: infrarust_api::types::Component::text(format!(
                                    "Backend unreachable: {e}"
                                )),
                            },
                        )
                    } else {
                        let msg = server_config.effective_disconnect_message();
                        client.disconnect(msg, &services.packet_registry).await.ok();
                        return Ok(InitialMode::Denied);
                    }
                } else {
                    tracing::warn!(
                        server = %routing.config_id,
                        error = %e,
                        "backend unreachable, sending disconnect to client"
                    );
                    let msg = server_config.effective_disconnect_message();
                    client.disconnect(msg, &services.packet_registry).await.ok();
                    return Ok(InitialMode::Denied);
                }
            }
        }
    };

    if matches!(mode, ConnectionMode::Backend(_)) {
        services.event_bus.fire_and_forget_arc(
            infrarust_api::events::connection::ServerConnectedEvent {
                player_id,
                server: target_server_id.clone(),
            },
        );
    }

    Ok(InitialMode::Connected {
        mode: Box::new(mode),
        server_id: target_server_id,
    })
}

#[allow(clippy::too_many_arguments)]
async fn connect_to_backend(
    client: &mut ClientBridge,
    auth_result: &AuthResult,
    login_completed: bool,
    routing: &RoutingData,
    handshake: &HandshakeData,
    version: ProtocolVersion,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    connection_info: &infrarust_transport::ConnectionInfo,
) -> Result<BackendBridge, CoreError> {
    let server_config = &routing.server_config;

    let backend_conn = backend_connector
        .connect(
            &routing.config_id,
            &server_config.addresses,
            server_config.timeouts.as_ref().map(|t| t.connect),
            server_config.send_proxy_protocol,
            connection_info,
        )
        .await?;

    let mut backend = BackendBridge::new(backend_conn.into_stream(), version);

    if login_completed {
        let handler = services.resolve_forwarding_handler(server_config);
        let fwd_data = build_forwarding_data(auth_result, connection_info, version);

        if handler.modifies_handshake() {
            let mut hs = build_handshake_for_backend(handshake, server_config);
            handler.apply_handshake(&mut hs, &fwd_data);
            backend
                .send_handshake_and_login(&hs, &auth_result.username, &services.packet_registry)
                .await?;
        } else {
            backend
                .send_initial_packets_offline(
                    handshake,
                    server_config,
                    &auth_result.username,
                    &services.packet_registry,
                )
                .await?;
        }

        let velocity_ctx = services.forwarding_secret().map(|s| (&fwd_data, s));

        if let Err(e) = backend
            .consume_backend_login(&services.packet_registry, version, velocity_ctx)
            .await
        {
            client
                .disconnect("Backend refused connection", &services.packet_registry)
                .await
                .ok();
            return Err(e);
        }

        if version.no_less_than(ProtocolVersion::V1_20_2) {
            let ack = SLoginAcknowledged;
            backend.send_packet(&ack, &services.packet_registry).await?;
            backend.set_state(ConnectionState::Config);
            tracing::debug!("backend LoginAcknowledged -> Config");
        }
    } else {
        backend
            .send_initial_packets(handshake, server_config)
            .await?;
    }

    Ok(backend)
}

fn build_forwarding_data(
    auth_result: &AuthResult,
    connection_info: &infrarust_transport::ConnectionInfo,
    protocol_version: ProtocolVersion,
) -> ForwardingData {
    let real_ip = connection_info
        .real_ip
        .unwrap_or(connection_info.peer_addr.ip());

    ForwardingData {
        real_ip,
        uuid: auth_result.player_uuid,
        username: auth_result.username.clone(),
        properties: auth_result.api_profile.properties.clone(),
        protocol_version,
        chat_session: None,
    }
}

async fn prepare_client_for_limbo(
    client: &mut ClientBridge,
    auth_result: &AuthResult,
    login_completed: &mut bool,
    version: ProtocolVersion,
    services: &ProxyServices,
) -> Result<(), CoreError> {
    ensure_login_complete_for_limbo(client, auth_result, login_completed, version, services)
        .await?;

    if version.no_less_than(ProtocolVersion::V1_20_2)
        && let Err(e) = crate::limbo::login::complete_config_for_limbo(
            client,
            version,
            &services.packet_registry,
            &services.registry_codec_cache,
        )
        .await
    {
        tracing::warn!("limbo config phase failed: {e}");
        client
            .disconnect(&e.to_string(), &services.packet_registry)
            .await
            .ok();
        return Err(e);
    }

    Ok(())
}

async fn ensure_login_complete_for_limbo(
    client: &mut ClientBridge,
    auth_result: &AuthResult,
    login_completed: &mut bool,
    version: ProtocolVersion,
    services: &ProxyServices,
) -> Result<(), CoreError> {
    if *login_completed {
        return Ok(());
    }

    super::auth::send_login_success(
        client,
        auth_result.player_uuid,
        &auth_result.username,
        &[],
        version,
        &services.packet_registry,
    )
    .await?;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        super::auth::consume_login_acknowledged(client, version, &services.packet_registry).await?;
    } else {
        client.set_state(ConnectionState::Play);
    }

    *login_completed = true;
    Ok(())
}
