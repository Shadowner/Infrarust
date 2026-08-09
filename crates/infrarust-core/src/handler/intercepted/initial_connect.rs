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
use crate::middleware::backend_selection::BackendTargets;
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
    backend_targets: Option<&BackendTargets>,
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
    let mut target_server_id = match choose.result() {
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
                        target_server: target_server_id.clone(),
                    },
                ));
            }
            infrarust_api::events::connection::ServerPreConnectResult::ConnectTo(id) => {
                target_server_id = id.clone();
            }
            _ => {}
        }
    }

    let redirected = if target_server_id.as_str() == routing.config_id {
        None
    } else {
        let Some((server_config, load_balancer)) = services
            .domain_router
            .find_route_by_server_id(target_server_id.as_str())
        else {
            tracing::warn!(
                from = %routing.config_id,
                to = %target_server_id,
                "plugin redirected to an unknown server"
            );
            client
                .disconnect("Unknown server", &services.packet_registry)
                .await
                .ok();
            return Ok(InitialMode::Denied);
        };
        Some(RoutingData {
            server_config,
            config_id: target_server_id.to_string(),
            load_balancer,
        })
    };

    let routing = redirected.as_ref().unwrap_or(routing);
    let server_config = &routing.server_config;

    let redirected_targets = redirected.as_ref().map(|r| BackendTargets {
        addresses: crate::loadbalancer::select_backend_addresses(
            &r.server_config,
            r.load_balancer.as_ref(),
            services.pending_backends.as_ref(),
            services.backend_health.as_ref(),
        ),
    });
    let backend_targets = redirected_targets.as_ref().or(backend_targets);

    if initial_mode.is_none() && !server_config.limbo_handlers.is_empty() {
        prepare_client_for_limbo(client, auth_result, login_completed, version, services).await?;
        if let Some(handlers) = resolve_limbo_lenient(
            &services.limbo_handler_registry,
            &server_config.limbo_handlers,
        ) {
            initial_mode = Some(ConnectionMode::Limbo(
                handlers,
                LimboEntryContext::InitialConnection {
                    target_server: target_server_id.clone(),
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
            backend_targets,
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
    backend_targets: Option<&BackendTargets>,
    version: ProtocolVersion,
    services: &ProxyServices,
    backend_connector: &BackendConnector,
    connection_info: &infrarust_transport::ConnectionInfo,
) -> Result<BackendBridge, CoreError> {
    let server_config = &routing.server_config;

    let addresses = BackendTargets::addresses_or_config(backend_targets, server_config);

    let backend_conn = backend_connector
        .connect(
            &routing.config_id,
            &addresses,
            server_config.timeouts.as_ref().map(|t| t.connect),
            server_config.send_proxy_protocol,
            connection_info,
        )
        .await?;

    let connected_address = backend_conn.server_address().clone();
    let mut backend = BackendBridge::new(backend_conn.into_stream(), version)
        .with_server_address(connected_address);

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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use infrarust_api::event::EventPriority;
    use infrarust_api::event::bus::{EventBus, EventBusExt};
    use infrarust_api::events::connection::{
        PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult,
    };
    use infrarust_api::types::ServerId;
    use infrarust_config::ServerConfig;
    use tokio::net::TcpListener;

    use crate::pipeline::types::ConnectionIntent;

    use crate::limbo::test_helpers::{test_client_bridge, test_profile, test_proxy_services};

    fn config(name: &str, address: &str) -> ServerConfig {
        toml::from_str(&format!(
            "name = \"{name}\"\ndomains = [\"{name}.test\"]\naddresses = [\"{address}\"]\nproxy_mode = \"offline\"\n"
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn redirect_connects_to_the_redirected_server() {
        let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin_listener.local_addr().unwrap();
        let target_addr = target_listener.local_addr().unwrap();

        let services = test_proxy_services();
        services.domain_router.add(
            crate::provider::ProviderId::file("origin"),
            config("origin", &origin_addr.to_string()),
        );
        services.domain_router.add(
            crate::provider::ProviderId::file("target"),
            config("target", &target_addr.to_string()),
        );

        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe::<PlayerChooseInitialServerEvent, _>(
            EventPriority::NORMAL,
            |event: &mut PlayerChooseInitialServerEvent| {
                event.set_result(PlayerChooseInitialServerResult::Redirect(ServerId::new(
                    "target",
                )));
            },
        );

        let version = ProtocolVersion::V1_20_2;
        let (mut client, _client_stream) = test_client_bridge(version).await;
        let (origin_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("origin")
            .unwrap();

        let auth_result = AuthResult {
            player_id: infrarust_api::types::PlayerId::new(1),
            player_uuid: uuid::Uuid::nil(),
            username: "Redirected".to_string(),
            api_profile: test_profile(),
            login_completed: false,
            online_mode: false,
        };
        let handshake = HandshakeData {
            domain: "origin.test".to_string(),
            port: 25565,
            protocol_version: version,
            intent: ConnectionIntent::Login,
            raw_packets: vec![],
        };
        let peer_addr = "127.0.0.1:40000".parse().unwrap();
        let connection_info = infrarust_transport::ConnectionInfo {
            peer_addr,
            real_ip: None,
            real_port: None,
            local_addr: peer_addr,
            connected_at: tokio::time::Instant::now(),
        };
        // Stale pipeline targets: they still point at the origin server.
        let stale_targets = BackendTargets {
            addresses: smallvec::smallvec![origin_config.addresses[0].address.clone()],
        };

        let mode = resolve_initial_mode(
            &mut client,
            &auth_result,
            &mut false,
            &RoutingData {
                server_config: origin_config,
                config_id: "origin".to_string(),
                load_balancer,
            },
            &handshake,
            Some(&stale_targets),
            version,
            &services,
            &BackendConnector::new(
                std::time::Duration::from_secs(2),
                infrarust_config::KeepaliveConfig::default(),
            ),
            &connection_info,
        )
        .await
        .unwrap();

        let InitialMode::Connected { mode, server_id } = mode else {
            panic!("expected a backend connection");
        };
        assert_eq!(server_id.as_str(), "target");
        let ConnectionMode::Backend(backend) = *mode else {
            panic!("expected backend mode, got limbo");
        };
        assert_eq!(
            backend.server_address().map(|a| a.port),
            Some(target_addr.port()),
            "the socket must reach the redirected server"
        );
    }
}
