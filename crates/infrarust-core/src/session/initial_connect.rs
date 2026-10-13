//! Initial server resolution and backend connection for intercepted modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::ConnectCause;
use infrarust_api::events::lifecycle::DisconnectCause;
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::{HANDLER_UNAVAILABLE, LimboHandler};
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::version::ProtocolVersion;

use super::session_loop::Pending;
use crate::error::CoreError;
use crate::forwarding::ForwardingHandler;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::loadbalancer::PendingTicket;
use crate::middleware::backend_selection::BackendTargets;
use crate::pipeline::types::RoutingData;
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::{Login, connect_backend};
use crate::session::client_bridge::ClientBridge;
use crate::session::client_login::complete_login;
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
    pub(crate) online_mode: bool,
}

impl LoginProgress {
    const fn bridges_backend_login(&self, handler: &ForwardingHandler) -> bool {
        !self.completed
            && (self.online_mode || self.rewritten || !matches!(handler, ForwardingHandler::None))
    }
}

fn kicked(reason: Component) -> InitialMode {
    InitialMode::Denied(DisconnectCause::Kicked {
        reason: Some(reason),
    })
}

fn resolve_limbo(
    registry: &LimboHandlerRegistry,
    names: &[String],
) -> Result<Vec<Arc<dyn LimboHandler>>, &'static str> {
    match registry.resolve_handlers(names) {
        Ok(handlers) if !handlers.is_empty() => Ok(handlers),
        Ok(_) => {
            tracing::warn!("limbo requested at initial connect but no handlers are configured");
            Err("No limbo handlers configured")
        }
        Err(e) => {
            tracing::warn!(error = %e, "refusing the player: a limbo handler is not loaded");
            Err(HANDLER_UNAVAILABLE)
        }
    }
}

async fn deny_limbo(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    reason: &str,
) -> Result<InitialMode, CoreError> {
    let reason = Component::text(reason);
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
            prepare_client_for_limbo(ctx, io, progress, &initial_server).await?;
            let handlers = match resolve_limbo(&services.limbo_handler_registry, limbo_handlers) {
                Ok(handlers) => handlers,
                Err(reason) => return deny_limbo(ctx, &mut io.client, reason).await,
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
            io.client.disconnect(reason, ctx.registry()).await.ok();
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
                io.client.disconnect(reason, ctx.registry()).await.ok();
                return Ok(kicked(reason.clone()));
            }
            infrarust_api::events::connection::ServerPreConnectResult::SendToLimbo {
                limbo_handlers,
            } => {
                prepare_client_for_limbo(ctx, io, progress, &target_server_id).await?;
                let handler_names = if limbo_handlers.is_empty() {
                    server_config.limbo_handlers.clone()
                } else {
                    limbo_handlers.clone()
                };
                let handlers = match resolve_limbo(&services.limbo_handler_registry, &handler_names)
                {
                    Ok(handlers) => handlers,
                    Err(reason) => return deny_limbo(ctx, &mut io.client, reason).await,
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
            io.client.disconnect(&reason, ctx.registry()).await.ok();
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

    if initial_mode.is_none() && !server_config.limbo_handlers.is_empty() {
        let handlers = match resolve_limbo(
            &services.limbo_handler_registry,
            &server_config.limbo_handlers,
        ) {
            Ok(handlers) => handlers,
            Err(reason) => return deny_limbo(ctx, &mut io.client, reason).await,
        };
        prepare_client_for_limbo(ctx, io, progress, &target_server_id).await?;
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
        let bridged = progress.bridges_backend_login(&forwarding_handler);

        let mut join = ServerJoin::new(player, target_server_id.clone());
        let addresses = BackendTargets::addresses_or_config(backend_targets, server_config);
        let login = if progress.completed {
            Login::Proxied
        } else if bridged {
            Login::Bridged(&mut *io)
        } else {
            Login::Relay
        };
        match connect_backend(ctx, &routing.config_id, server_config, &addresses, login).await {
            Ok(backend) => {
                progress.completed |= bridged;
                if progress.completed {
                    join.connected(&services.event_bus).await;
                }
                pending = Pending::join(join);
                ConnectionMode::Backend(backend)
            }
            Err(CoreError::ClientLeft) => {
                return Ok(InitialMode::Denied(DisconnectCause::ClientQuit));
            }
            Err(e) => {
                if !ctx.token.is_cancelled() {
                    tracing::info!(
                        server = %routing.config_id,
                        error = %e,
                        "initial backend connection failed"
                    );
                }
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
    io: &mut SessionIo,
    progress: &mut LoginProgress,
    server: &ServerId,
) -> Result<(), CoreError> {
    ensure_login_complete(ctx, io, progress).await?;

    let client = &mut io.client;
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
    io: &mut SessionIo,
    progress: &mut LoginProgress,
) -> Result<(), CoreError> {
    if progress.completed {
        return Ok(());
    }

    complete_login(ctx, io).await?;

    progress.completed = true;
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
    use infrarust_protocol::packets::config::{
        CFinishConfig, CKnownPacks, SAcknowledgeFinishConfig, SKnownPacks,
    };
    use infrarust_protocol::packets::login::{
        CLoginDisconnect, CLoginPluginRequest, SLoginPluginResponse,
    };
    use infrarust_protocol::version::ConnectionState;
    use tokio::net::TcpListener;

    use crate::forwarding::{ForwardingHandler, ForwardingMode, build_forwarding_handler};
    use crate::loadbalancer::AddressConnectionCount;
    use crate::player::PlayerSession;
    use crate::session::context::test_helpers::{test_connector, test_context, test_io};
    use crate::session::proxy_loop::ProxyLoopOutcome;
    use crate::session::test_peer::{
        V, backend_login, client_pair, finish_backend_login, finish_client_login, plugin_request,
        plugin_response, server_config, var_int, velocity_services, with_handshake,
    };

    use crate::limbo::test_helpers::{test_client_bridge, test_proxy_services};

    const fn offline_login() -> LoginProgress {
        LoginProgress {
            completed: false,
            rewritten: false,
            online_mode: false,
        }
    }

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

        let mut progress = offline_login();
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
        let mut progress = offline_login();
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

    #[tokio::test]
    async fn a_configured_limbo_handler_that_is_not_loaded_kicks_the_player() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = test_proxy_services();
        let mut server = config("origin", &backend.local_addr().unwrap().to_string());
        server.limbo_handlers = vec!["auth".to_string(), "lobby".to_string()];
        services
            .domain_router
            .add(crate::provider::ProviderId::file("origin"), server);
        services
            .limbo_handler_registry
            .register(
                "p",
                Box::new(crate::limbo::test_helpers::FixedHandler {
                    name: "lobby",
                    result: infrarust_api::limbo::handler::HandlerResult::Accept,
                }),
            )
            .unwrap();

        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = test_context(&services, &connector, player);
        let (client, _client_stream) = test_client_bridge(ctx.version()).await;
        let mut io = test_io(&ctx, client);
        let (server_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("origin")
            .unwrap();
        let mut progress = offline_login();
        let mode = resolve_initial_mode(
            &ctx,
            &mut io,
            &RoutingData {
                server_config,
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
            panic!("a missing configured limbo handler must kick the player");
        };
        assert_eq!(reason.to_plain(), HANDLER_UNAVAILABLE);
    }

    fn origin(
        services: &crate::services::ProxyServices,
        backend: &TcpListener,
        extra: &str,
    ) -> RoutingData {
        services.domain_router.add(
            crate::provider::ProviderId::file("origin"),
            server_config(backend, extra),
        );
        let (server_config, load_balancer) = services
            .domain_router
            .find_route_by_server_id("origin")
            .unwrap();
        RoutingData {
            server_config,
            config_id: "origin".to_string(),
            load_balancer,
        }
    }

    async fn resolve(
        ctx: &SessionContext<'_>,
        io: &mut SessionIo,
        routing: &RoutingData,
        mut progress: LoginProgress,
    ) -> InitialMode {
        resolve_initial_mode(ctx, io, routing, None, &mut None, &mut progress)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_custom_login_request_reaches_the_client_and_its_answer_reaches_the_backend() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = velocity_services();
        let routing = origin(&services, &backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            let request = conn
                .send(&plugin_request(-100, "example:handshake", b"hello"))
                .await;
            let answer = conn.frame().await;
            finish_backend_login(&mut conn).await;
            (request, answer, conn)
        });
        let client_side = tokio::spawn(async move {
            let request = peer.frame().await;
            let answer = peer.send(&plugin_response(-100, b"ready")).await;
            let threshold = finish_client_login(&mut peer).await;
            (request, answer, threshold, peer)
        });

        let mode = resolve(&ctx, &mut io, &routing, offline_login()).await;
        let (sent_request, received_answer, _conn) = backend_side.await.unwrap();
        let (received_request, sent_answer, threshold, _peer) = client_side.await.unwrap();

        assert_eq!(received_request.id, sent_request.id);
        assert_eq!(received_request.payload, sent_request.payload);
        assert_eq!(received_request.payload[..5], var_int(-100)[..]);
        assert_eq!(received_answer.id, sent_answer.id);
        assert_eq!(received_answer.payload, sent_answer.payload);
        assert_eq!(threshold, 256);
        let InitialMode::Connected(initial) = mode else {
            panic!("expected a backend connection");
        };
        assert!(matches!(initial.mode, ConnectionMode::Backend(_)));
        assert_eq!(io.client.state(), ConnectionState::Config);
        assert_eq!(io.client.compression_threshold(), Some(256));
    }

    #[tokio::test]
    async fn an_online_login_without_forwarding_is_bridged() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = test_proxy_services();
        let routing = origin(&services, &backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(3, "fml:loginwrapper", b"mods"))
                .await;
            let answer = conn.expect::<SLoginPluginResponse>().await;
            finish_backend_login(&mut conn).await;
            (answer, conn)
        });
        let client_side = tokio::spawn(async move {
            let request = peer.expect::<CLoginPluginRequest>().await;
            peer.send(&plugin_response(request.message_id.0, b"ok"))
                .await;
            finish_client_login(&mut peer).await;
            (request, peer)
        });

        let online = LoginProgress {
            online_mode: true,
            ..offline_login()
        };
        let mode = resolve(&ctx, &mut io, &routing, online).await;
        let (answer, _conn) = backend_side.await.unwrap();
        let (request, _peer) = client_side.await.unwrap();

        assert_eq!(request.channel, "fml:loginwrapper");
        assert_eq!(answer.message_id.0, 3);
        assert_eq!(answer.data, b"ok");
        assert!(matches!(mode, InitialMode::Connected(_)));
    }

    #[tokio::test]
    async fn offline_bungeecord_forwarding_reaches_the_backend() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = test_proxy_services();
        let routing = origin(&services, &backend, "forwarding_mode = \"bungee_cord\"");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let uuid = player.game_profile().uuid;
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, handshake) = backend_login(&backend).await;
            finish_backend_login(&mut conn).await;
            (handshake, conn)
        });
        let client_side = tokio::spawn(async move {
            finish_client_login(&mut peer).await;
            peer
        });

        let mode = resolve(&ctx, &mut io, &routing, offline_login()).await;
        let (handshake, _conn) = backend_side.await.unwrap();
        let _peer = client_side.await.unwrap();

        assert_eq!(
            handshake.server_address,
            format!("origin.test\0127.0.0.1\0{}\0[]", uuid.as_simple())
        );
        assert!(matches!(mode, InitialMode::Connected(_)));
    }

    #[tokio::test]
    async fn a_backend_kick_during_the_bridge_reaches_the_client_as_a_login_disconnect() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = velocity_services();
        let routing = origin(&services, &backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);
        let reason = Component::text("Missing mods: create");

        let json = reason.to_json();
        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(1, "fml:loginwrapper", b"mods"))
                .await;
            conn.expect::<SLoginPluginResponse>().await;
            conn.send(&CLoginDisconnect { reason: json }).await;
            conn
        });
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginPluginRequest>().await;
            peer.send(&plugin_response(1, b"")).await;
            let kicked = peer.expect::<CLoginDisconnect>().await;
            (kicked, peer)
        });

        let InitialMode::Connected(initial) =
            resolve(&ctx, &mut io, &routing, offline_login()).await
        else {
            panic!("a refused bridge must leave the kick to the session loop");
        };
        assert!(matches!(initial.mode, ConnectionMode::Kicked(_)));
        let outcome = crate::session::session_loop::run_session_loop(&ctx, &mut io, *initial).await;
        let _conn = backend_side.await.unwrap();
        let (kicked, _peer) = client_side.await.unwrap();

        assert_eq!(
            crate::util::text::decode_text_component(
                kicked.reason.as_bytes(),
                V,
                ConnectionState::Login
            ),
            reason
        );
        assert!(
            matches!(outcome, ProxyLoopOutcome::BackendClosed { reason: Some(ref r) } if *r == reason),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_client_that_leaves_during_the_bridge_ends_the_session_quietly() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = velocity_services();
        let routing = origin(&services, &backend, "");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let backend_side = tokio::spawn(async move {
            let (mut conn, _) = backend_login(&backend).await;
            conn.send(&plugin_request(-100, "example:handshake", b"hello"))
                .await;
            conn.recv().await.is_none()
        });
        let client_side = tokio::spawn(async move {
            peer.expect::<CLoginPluginRequest>().await;
            drop(peer);
        });

        let mode = resolve(&ctx, &mut io, &routing, offline_login()).await;
        client_side.await.unwrap();

        assert!(
            matches!(mode, InitialMode::Denied(DisconnectCause::ClientQuit)),
            "a client that leaves mid-login has quit"
        );
        assert!(
            backend_side.await.unwrap(),
            "the backend connection must be closed"
        );
    }

    #[tokio::test]
    async fn the_limbo_path_completes_the_login_with_compression() {
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let services = test_proxy_services();
        services
            .limbo_handler_registry
            .register(
                "p",
                Box::new(crate::limbo::test_helpers::FixedHandler {
                    name: "lobby",
                    result: infrarust_api::limbo::handler::HandlerResult::Accept,
                }),
            )
            .unwrap();
        let routing = origin(&services, &backend, "limbo_handlers = [\"lobby\"]");
        let connector = test_connector();
        let (player, _commands) = PlayerSession::new_test(true);
        let ctx = with_handshake(test_context(&services, &connector, player));
        let (client, mut peer) = client_pair(V).await;
        let mut io = test_io(&ctx, client);

        let client_side = tokio::spawn(async move {
            let threshold = finish_client_login(&mut peer).await;
            let known_packs = peer.id::<CKnownPacks>();
            let finish = peer.id::<CFinishConfig>();
            loop {
                let frame = peer.frame().await;
                if frame.id == known_packs {
                    peer.send(&SKnownPacks { packs: vec![] }).await;
                }
                if frame.id == finish {
                    peer.send(&SAcknowledgeFinishConfig).await;
                    break;
                }
            }
            (threshold, peer)
        });

        let online = LoginProgress {
            online_mode: true,
            ..offline_login()
        };
        let mode = resolve(&ctx, &mut io, &routing, online).await;
        let (threshold, _peer) = client_side.await.unwrap();

        assert_eq!(threshold, 256);
        let InitialMode::Connected(initial) = mode else {
            panic!("expected the limbo");
        };
        assert!(matches!(initial.mode, ConnectionMode::Limbo(..)));
        assert_eq!(io.client.state(), ConnectionState::Play);
        assert_eq!(io.client.compression_threshold(), Some(256));
    }

    #[test]
    fn every_forwarding_mode_bridges_the_offline_login() {
        for mode in [
            ForwardingMode::BungeeCord,
            ForwardingMode::BungeeGuard {
                token: "token".to_string(),
            },
            ForwardingMode::Velocity {
                secret: b"test-secret".to_vec(),
            },
        ] {
            let handler = build_forwarding_handler(&mode);
            assert!(offline_login().bridges_backend_login(&handler), "{mode:?}");
            let completed = LoginProgress {
                completed: true,
                ..offline_login()
            };
            assert!(!completed.bridges_backend_login(&handler), "{mode:?}");
        }
        assert!(!offline_login().bridges_backend_login(&ForwardingHandler::None));
    }

    #[test]
    fn online_mode_and_a_rewritten_profile_bridge_the_login() {
        let online = LoginProgress {
            online_mode: true,
            ..offline_login()
        };
        let rewritten = LoginProgress {
            rewritten: true,
            ..offline_login()
        };
        for progress in [online, rewritten] {
            assert!(progress.bridges_backend_login(&ForwardingHandler::None));
            let completed = LoginProgress {
                completed: true,
                ..progress
            };
            assert!(!completed.bridges_backend_login(&ForwardingHandler::None));
        }
    }
}
