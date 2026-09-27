//! Initial server resolution and backend connection for intercepted modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::events::lifecycle::DisconnectCause;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::version::ProtocolVersion;

use super::session_loop::Pending;
use crate::error::CoreError;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::loadbalancer::PendingTicket;
use crate::middleware::backend_selection::BackendTargets;
use crate::pipeline::types::RoutingData;
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::{Login, connect_backend};
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::Kick;
use crate::session::server_join::{ServerJoin, pre_connect};
use crate::session::wake::wake;

pub(crate) enum ConnectionMode {
    Backend(BackendBridge),
    Limbo(Vec<Arc<dyn LimboHandler>>, LimboEntryContext),
    Kicked(Kick),
}

pub(crate) struct Initial {
    pub(crate) mode: ConnectionMode,
    pub(crate) server: ServerId,
    pub(crate) pending: Pending,
}

pub(crate) enum InitialMode {
    Connected(Box<Initial>),
    /// Disconnect already sent.
    Denied(DisconnectCause),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LoginProgress {
    pub(crate) completed: bool,
    pub(crate) rewritten: bool,
}

fn kicked(reason: Component) -> InitialMode {
    InitialMode::Denied(DisconnectCause::Kicked {
        reason: Some(reason),
    })
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
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
) -> Result<InitialMode, CoreError> {
    tracing::warn!("SendToLimbo at initial connect but no handlers resolved");
    let reason = Component::text("No limbo handlers configured");
    client.disconnect(&reason, ctx.registry()).await.ok();
    Ok(kicked(reason))
}

pub(crate) async fn resolve_initial_mode(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    routing: &RoutingData,
    backend_targets: Option<&BackendTargets>,
    pending_ticket: &mut Option<PendingTicket>,
    progress: &mut LoginProgress,
) -> Result<InitialMode, CoreError> {
    let services = ctx.services;
    let player = &ctx.session;
    let client = &mut io.client;
    let server_config = &routing.server_config;

    let initial_server = ServerId::new(routing.config_id.clone());
    let choose = infrarust_api::events::connection::PlayerChooseInitialServerEvent::new(
        ctx.player(),
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
            prepare_client_for_limbo(ctx, client, progress, &initial_server).await?;
            let Some(handlers) =
                resolve_limbo_strict(&services.limbo_handler_registry, limbo_handlers)
            else {
                return deny_no_limbo_handlers(ctx, client).await;
            };
            initial_mode = Some(ConnectionMode::Limbo(
                handlers,
                LimboEntryContext::InitialConnection {
                    target_server: initial_server.clone(),
                },
            ));
            initial_server.clone()
        }
        infrarust_api::events::connection::PlayerChooseInitialServerResult::Denied { reason } => {
            client.disconnect(reason, ctx.registry()).await.ok();
            return Ok(kicked(reason.clone()));
        }
        _ => initial_server.clone(),
    };

    let approved = initial_mode.is_none();
    if approved {
        let pre_connect = pre_connect(
            &services.event_bus,
            player,
            target_server_id.clone(),
            ConnectCause::Initial,
        )
        .await;
        match pre_connect.result() {
            infrarust_api::events::connection::ServerPreConnectResult::Allowed => {}
            infrarust_api::events::connection::ServerPreConnectResult::Denied { reason } => {
                client.disconnect(reason, ctx.registry()).await.ok();
                return Ok(kicked(reason.clone()));
            }
            infrarust_api::events::connection::ServerPreConnectResult::SendToLimbo {
                limbo_handlers,
            } => {
                prepare_client_for_limbo(ctx, client, progress, &target_server_id).await?;
                let handler_names = if limbo_handlers.is_empty() {
                    server_config.limbo_handlers.clone()
                } else {
                    limbo_handlers.clone()
                };
                let Some(handlers) =
                    resolve_limbo_lenient(&services.limbo_handler_registry, &handler_names)
                else {
                    return deny_no_limbo_handlers(ctx, client).await;
                };
                initial_mode = Some(ConnectionMode::Limbo(
                    handlers,
                    LimboEntryContext::InitialConnection {
                        target_server: target_server_id.clone(),
                    },
                ));
            }
            infrarust_api::events::connection::ServerPreConnectResult::Redirect(id) => {
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
            let reason = Component::text("Unknown server");
            client.disconnect(&reason, ctx.registry()).await.ok();
            return Ok(kicked(reason));
        };
        Some(RoutingData {
            server_config,
            config_id: target_server_id.to_string(),
            load_balancer,
        })
    };

    let routing = redirected.as_ref().unwrap_or(routing);
    let server_config = &routing.server_config;

    let redirected_targets = redirected
        .as_ref()
        .map(|r| fresh_targets(r, ctx, pending_ticket));
    let backend_targets = redirected_targets.as_ref().or(backend_targets);

    if initial_mode.is_none()
        && !server_config.limbo_handlers.is_empty()
        && let Some(handlers) = resolve_limbo_lenient(
            &services.limbo_handler_registry,
            &server_config.limbo_handlers,
        )
    {
        prepare_client_for_limbo(ctx, client, progress, &target_server_id).await?;
        initial_mode = Some(ConnectionMode::Limbo(
            handlers,
            LimboEntryContext::InitialConnection {
                target_server: target_server_id.clone(),
            },
        ));
    }

    let mut pending = if approved {
        Pending::approved(target_server_id.clone())
    } else {
        Pending::nothing()
    };
    let woken = if initial_mode.is_none() {
        wake(services, server_config, player.shutdown_token()).await
    } else {
        Ok(false)
    };
    let mode = if let Some(limbo_mode) = initial_mode {
        limbo_mode
    } else if let Err(unavailable) = woken {
        tracing::info!(
            server = %routing.config_id,
            reason = ?unavailable,
            "the server manager could not start the initial server"
        );
        pending = Pending::nothing();
        ConnectionMode::Kicked(unavailable.into_kick(target_server_id.clone()))
    } else {
        let warmed = matches!(woken, Ok(true)).then(|| fresh_targets(routing, ctx, pending_ticket));
        let backend_targets = warmed.as_ref().or(backend_targets);

        let forwarding_handler = services.resolve_forwarding_handler(server_config);
        if requires_proxy_completed_login(
            &forwarding_handler,
            progress.rewritten,
            progress.completed,
        ) {
            ensure_login_complete(ctx, client, progress).await?;
        }

        let mut join = ServerJoin::new(player, target_server_id.clone());
        let addresses = BackendTargets::addresses_or_config(backend_targets, server_config);
        let login = if progress.completed {
            Login::Proxied
        } else {
            Login::Relay
        };
        match connect_backend(ctx, &routing.config_id, server_config, &addresses, login).await {
            Ok(backend) => {
                if progress.completed {
                    join.connected(&services.event_bus).await;
                }
                pending = Pending::join(join);
                ConnectionMode::Backend(backend)
            }
            Err(e) => {
                tracing::info!(
                    server = %routing.config_id,
                    error = %e,
                    "initial backend connection failed"
                );
                pending = Pending::nothing();
                ConnectionMode::Kicked(Kick::failed(target_server_id.clone(), e, false))
            }
        }
    };

    Ok(InitialMode::Connected(Box::new(Initial {
        mode,
        server: target_server_id,
        pending,
    })))
}

fn fresh_targets(
    routing: &RoutingData,
    ctx: &SessionContext<'_>,
    pending_ticket: &mut Option<PendingTicket>,
) -> BackendTargets {
    let services = ctx.services;
    let targets = BackendTargets {
        addresses: crate::loadbalancer::select_backend_addresses(
            &routing.server_config,
            routing.load_balancer.as_ref(),
            services.pending_backends.as_ref(),
            services.backend_health.as_ref(),
        ),
    };
    if let Some(picked) = targets.addresses.first() {
        *pending_ticket = Some(services.pending_backends.reserve(picked));
    }
    targets
}

async fn prepare_client_for_limbo(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    progress: &mut LoginProgress,
    server: &ServerId,
) -> Result<(), CoreError> {
    ensure_login_complete(ctx, client, progress).await?;

    let version = ctx.version();
    if version.no_less_than(ProtocolVersion::V1_20_2)
        && let Err(e) = crate::limbo::login::complete_config_for_limbo(
            client,
            version,
            ctx.registry(),
            &ctx.services.registry_codec_cache,
            Some(&FrameChain::new(ctx, server)),
        )
        .await
    {
        tracing::warn!("limbo config phase failed: {e}");
        client
            .disconnect(&Component::text(e.to_string()), ctx.registry())
            .await
            .ok();
        return Err(e);
    }

    Ok(())
}

async fn ensure_login_complete(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    progress: &mut LoginProgress,
) -> Result<(), CoreError> {
    if progress.completed {
        return Ok(());
    }

    crate::session::client_login::complete_login(
        client,
        ctx.profile(),
        ctx.version(),
        ctx.registry(),
    )
    .await?;

    progress.completed = true;
    Ok(())
}

const fn requires_proxy_completed_login(
    handler: &crate::forwarding::ForwardingHandler,
    profile_rewritten: bool,
    login_completed: bool,
) -> bool {
    !login_completed && (handler.is_velocity() || profile_rewritten)
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

    use crate::forwarding::{ForwardingHandler, ForwardingMode, build_forwarding_handler};
    use crate::loadbalancer::AddressConnectionCount;
    use crate::player::PlayerSession;
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};

    use crate::limbo::test_helpers::{test_client_bridge, test_proxy_services};

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

        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = test_context(&services, &connector, player);
        let (client, _client_stream) = test_client_bridge(ctx.version()).await;
        let mut io = test_io(&ctx, client);
        let (origin_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("origin")
            .unwrap();

        // Stale pipeline targets: they still point at the origin server.
        let stale_targets = BackendTargets {
            addresses: smallvec::smallvec![origin_config.addresses[0].address.clone()],
        };

        let origin_address = origin_config.addresses[0].address.clone();
        let target_address: infrarust_config::ServerAddress =
            target_addr.to_string().parse().unwrap();
        let mut pending_ticket = Some(services.pending_backends.reserve(&origin_address));
        assert_eq!(
            services
                .pending_backends
                .active_connections_for_address(&origin_address),
            1
        );

        let mut progress = LoginProgress {
            completed: false,
            rewritten: false,
        };
        let mode = resolve_initial_mode(
            &ctx,
            &mut io,
            &RoutingData {
                server_config: origin_config,
                config_id: "origin".to_string(),
                load_balancer,
            },
            Some(&stale_targets),
            &mut pending_ticket,
            &mut progress,
        )
        .await
        .unwrap();

        let InitialMode::Connected(initial) = mode else {
            panic!("expected a backend connection");
        };
        assert_eq!(initial.server.as_str(), "target");
        let ConnectionMode::Backend(backend) = initial.mode else {
            panic!("expected backend mode, got limbo");
        };
        assert_eq!(
            backend.server_address().map(|a| a.port),
            Some(target_addr.port()),
            "the socket must reach the redirected server"
        );

        assert_eq!(
            services
                .pending_backends
                .active_connections_for_address(&origin_address),
            0,
            "the origin reservation must not survive the redirection"
        );
        assert_eq!(
            services
                .pending_backends
                .active_connections_for_address(&target_address),
            1,
            "the redirected address must be reserved until the session attaches"
        );
        drop(pending_ticket);
        assert_eq!(
            services
                .pending_backends
                .active_connections_for_address(&target_address),
            0
        );
    }

    #[tokio::test]
    async fn a_denied_initial_server_choice_kicks_the_player_before_any_backend() {
        let origin_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_addr = origin_listener.local_addr().unwrap();
        let services = test_proxy_services();
        services.domain_router.add(
            crate::provider::ProviderId::file("origin"),
            config("origin", &origin_addr.to_string()),
        );
        let bus: &dyn EventBus = services.event_bus.as_ref();
        bus.subscribe::<PlayerChooseInitialServerEvent, _>(
            EventPriority::NORMAL,
            |event: &mut PlayerChooseInitialServerEvent| {
                event.deny(Component::text("not today"));
            },
        );

        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = test_context(&services, &connector, player);
        let (client, _client_stream) = test_client_bridge(ctx.version()).await;
        let mut io = test_io(&ctx, client);
        let (origin_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("origin")
            .unwrap();
        let mut progress = LoginProgress {
            completed: false,
            rewritten: false,
        };
        let mode = resolve_initial_mode(
            &ctx,
            &mut io,
            &RoutingData {
                server_config: origin_config,
                config_id: "origin".to_string(),
                load_balancer,
            },
            None,
            &mut None,
            &mut progress,
        )
        .await
        .unwrap();

        let InitialMode::Denied(DisconnectCause::Kicked {
            reason: Some(reason),
        }) = mode
        else {
            panic!("a denied choice must kick the player");
        };
        assert_eq!(reason.to_plain(), "not today");
    }

    #[test]
    fn velocity_requires_proxy_completed_login_for_offline_flow() {
        let velocity = build_forwarding_handler(&ForwardingMode::Velocity {
            secret: b"test-secret".to_vec(),
        });
        assert!(requires_proxy_completed_login(&velocity, false, false));
        assert!(!requires_proxy_completed_login(&velocity, false, true));
        assert!(!requires_proxy_completed_login(
            &ForwardingHandler::None,
            false,
            false
        ));
    }

    #[test]
    fn a_rewritten_profile_requires_proxy_completed_login() {
        assert!(requires_proxy_completed_login(
            &ForwardingHandler::None,
            true,
            false
        ));
        assert!(!requires_proxy_completed_login(
            &ForwardingHandler::None,
            true,
            true
        ));
    }
}
