use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{
    DisconnectCause, GameProfileRequestEvent, LoginEvent, LoginResult, PreLoginEvent,
    PreLoginResult,
};
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{Component, GameProfile, ServerId};
use infrarust_config::{DomainRewrite, ProxyMode, ServerAddress, ServerConfig};
use infrarust_protocol::Packet;
use infrarust_protocol::io::PacketEncoder;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{
    BackendConnection, BackendConnector, ForwardEndReason, ForwardResult, select_forwarder,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::auth::game_profile::offline_profile_uuid;
use crate::error::CoreError;
use crate::forwarding::{ForwardingData, ForwardingHandler, build_handshake_for_backend};
use crate::loadbalancer::{PendingTicket, select_backend_addresses};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::player::lifecycle::PlayerLifecycle;
use crate::player::{PlayerCommand, PlayerSession, SHUTDOWN_REASON};
use crate::services::ProxyServices;
use crate::session::kick::Kick;
use crate::session::server_join::pre_connect;
use crate::session::wake::wake;

pub(crate) const LIMBO_UNAVAILABLE: &str = "Limbo is not available on this server";
pub(crate) const UNKNOWN_SERVER: &str = "Unknown server";
pub(crate) const PROXY_LOGIN_SERVER: &str = "This server cannot be joined from here";
const MAX_KICK_REDIRECTS: usize = 3;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Wire {
    Modern(ProtocolVersion),
    Legacy,
}

pub(crate) enum Opening<'a> {
    Modern(&'a HandshakeData),
    Legacy(&'a [u8]),
}

pub(crate) struct Arrival {
    pub(crate) username: String,
    pub(crate) claimed_uuid: Option<Uuid>,
    pub(crate) protocol_version: infrarust_api::types::ProtocolVersion,
    pub(crate) domain: String,
}

#[derive(Clone)]
pub(crate) struct Route {
    pub(crate) routing: RoutingData,
    pub(crate) addresses: Vec<ServerAddress>,
}

pub(crate) struct ForwardedLogin<'a> {
    pub(crate) services: &'a ProxyServices,
    pub(crate) connector: &'a BackendConnector,
    pub(crate) shutdown: &'a CancellationToken,
    pub(crate) wire: Wire,
}

struct Admitted {
    player: Arc<PlayerSession>,
    lifecycle: PlayerLifecycle,
    commands: mpsc::Receiver<PlayerCommand>,
    session_token: CancellationToken,
}

pub(crate) struct Ready {
    pub(crate) player: Arc<PlayerSession>,
    pub(crate) server: ServerId,
    mode: ProxyMode,
    backend: BackendConnection,
    lifecycle: PlayerLifecycle,
    commands: mpsc::Receiver<PlayerCommand>,
    session_token: CancellationToken,
    shutdown: CancellationToken,
}

impl Ready {
    pub(crate) async fn forward(mut self, client: TcpStream) -> ForwardResult {
        let forwarder = select_forwarder(self.mode);
        let result = forwarder
            .forward(
                client,
                self.backend.into_stream(),
                self.session_token.clone(),
            )
            .await;
        let cause = match &result.reason {
            ForwardEndReason::ClientClosed => DisconnectCause::ClientQuit,
            ForwardEndReason::BackendClosed => DisconnectCause::BackendClosed { reason: None },
            ForwardEndReason::Shutdown => {
                cancelled_cause(&self.shutdown, queued_kick(&mut self.commands))
            }
            _ => DisconnectCause::Error,
        };
        self.lifecycle.end(cause).await;
        result
    }
}

impl ForwardedLogin<'_> {
    pub(crate) async fn open(
        &self,
        ctx: &mut ConnectionContext,
        arrival: Arrival,
        origin: Route,
        opening: Opening<'_>,
    ) -> Result<Option<Ready>, CoreError> {
        let origin_server = ServerId::new(origin.routing.config_id.clone());
        let Some(mut admitted) = self.admit(ctx, arrival, origin_server).await? else {
            return Ok(None);
        };
        if let Some(cause) = self.interrupted(ctx, &mut admitted).await {
            admitted.lifecycle.end(cause).await;
            return Ok(None);
        }

        let choose = self
            .services
            .event_bus
            .fire(PlayerChooseInitialServerEvent::new(
                Arc::clone(&admitted.player) as Arc<dyn Player>,
                ServerId::new(origin.routing.config_id.clone()),
            ))
            .await;
        let target = match choose.result() {
            PlayerChooseInitialServerResult::Redirect(server) => server.clone(),
            PlayerChooseInitialServerResult::SendToLimbo { .. } => {
                self.refuse_limbo(ctx, admitted, "PlayerChooseInitialServerEvent")
                    .await;
                return Ok(None);
            }
            _ => choose.initial_server.clone(),
        };

        self.connect(ctx, admitted, target, &origin, &opening).await
    }

    async fn admit(
        &self,
        ctx: &mut ConnectionContext,
        arrival: Arrival,
        origin_server: ServerId,
    ) -> Result<Option<Admitted>, CoreError> {
        let services = self.services;
        let bus = &services.event_bus;
        let remote_addr = ctx.client_addr();
        let profile = GameProfile {
            uuid: offline_profile_uuid(
                services.config.auth.offline_uuid,
                &arrival.username,
                arrival.claimed_uuid,
            ),
            username: arrival.username,
            properties: vec![],
        };

        let pre_login = bus
            .fire(PreLoginEvent::new(
                profile.clone(),
                remote_addr,
                arrival.protocol_version,
                arrival.domain.clone(),
            ))
            .await;
        match pre_login.result() {
            PreLoginResult::Denied { reason } => {
                tracing::info!(username = %profile.username, "login denied by a plugin");
                self.kick(ctx, reason).await;
                return Ok(None);
            }
            PreLoginResult::ForceOffline | PreLoginResult::ForceOnline => {
                tracing::debug!(
                    username = %profile.username,
                    result = ?pre_login.result(),
                    "ignoring the PreLoginEvent authentication result, the backend runs this login"
                );
            }
            _ => {}
        }

        let request = bus
            .fire(GameProfileRequestEvent::new(
                profile,
                false,
                remote_addr,
                Some(arrival.domain.clone()),
                arrival.protocol_version,
            ))
            .await;
        let profile = request.profile;

        let domain = arrival.domain;
        let attempt =
            LoginAttempt::post_auth(ctx.client_ip, profile.username.clone(), profile.uuid, false)
                .virtual_host(domain.clone())
                .server(origin_server);
        if let Some(reason) = services.ban_manager.refusal(&attempt).await {
            self.kick(ctx, &reason).await;
            return Ok(None);
        }

        let session_token = self.shutdown.child_token();
        let (command_tx, commands) = PlayerSession::channel();
        let player = Arc::new(
            PlayerSession::new(
                crate::player::next_player_id(),
                profile,
                arrival.protocol_version,
                remote_addr,
                None,
                false,
                false,
                command_tx,
                session_token.clone(),
                crate::permissions::default_checker(),
                Arc::clone(&services.backend_load),
            )
            .with_permissions(Arc::clone(&services.permission_service))
            .with_virtual_host(domain),
        );

        player.setup_permissions(bus).await;

        let login = bus
            .fire(LoginEvent::new(
                Arc::clone(&player) as Arc<dyn Player>,
                false,
            ))
            .await;
        if let LoginResult::Denied { reason } = login.result() {
            tracing::info!(username = %player.profile().username, "login denied by a plugin");
            self.kick(ctx, reason).await;
            return Ok(None);
        }

        let lifecycle = PlayerLifecycle::begin(services, Arc::clone(&player)).await;
        Ok(Some(Admitted {
            player,
            lifecycle,
            commands,
            session_token,
        }))
    }

    async fn connect(
        &self,
        ctx: &mut ConnectionContext,
        mut admitted: Admitted,
        mut target: ServerId,
        origin: &Route,
        opening: &Opening<'_>,
    ) -> Result<Option<Ready>, CoreError> {
        let mut cause = ConnectCause::Initial;
        let mut redirects = 0;
        loop {
            let pre = pre_connect(&self.services.event_bus, &admitted.player, target, cause).await;
            let server = match pre.result() {
                ServerPreConnectResult::ConnectTo(server) => server.clone(),
                ServerPreConnectResult::Denied { reason } => {
                    let reason = reason.clone();
                    self.kick(ctx, &reason).await;
                    admitted
                        .lifecycle
                        .end(DisconnectCause::Kicked {
                            reason: Some(reason),
                        })
                        .await;
                    return Ok(None);
                }
                ServerPreConnectResult::SendToLimbo { .. } => {
                    self.refuse_limbo(ctx, admitted, "ServerPreConnectEvent")
                        .await;
                    return Ok(None);
                }
                _ => pre.server.clone(),
            };

            if let Some(ended) = self.interrupted(ctx, &mut admitted).await {
                admitted.lifecycle.end(ended).await;
                return Ok(None);
            }

            let mut route = match self.resolve(ctx, &server, origin) {
                Ok(route) => route,
                Err(reason) => {
                    let reason = Component::text(reason);
                    self.kick(ctx, &reason).await;
                    admitted
                        .lifecycle
                        .end(DisconnectCause::Kicked {
                            reason: Some(reason),
                        })
                        .await;
                    return Ok(None);
                }
            };

            let woken = wake(
                self.services,
                &route.routing.server_config,
                &admitted.session_token,
            )
            .await;
            if let Some(ended) = self.interrupted(ctx, &mut admitted).await {
                admitted.lifecycle.end(ended).await;
                return Ok(None);
            }
            let kick = match woken {
                Ok(warmed) => {
                    if warmed {
                        route.addresses = self.select(ctx, &route.routing);
                    }
                    match self.open_backend(ctx, &admitted, &route, opening).await {
                        Ok(backend) => {
                            self.services
                                .event_bus
                                .fire(ServerConnectedEvent::new(
                                    Arc::clone(&admitted.player) as Arc<dyn Player>,
                                    server.clone(),
                                    admitted.player.current_server(),
                                ))
                                .await;
                            admitted.player.set_current_server(server.clone());
                            ctx.extensions.remove::<PendingTicket>();
                            return Ok(Some(Ready {
                                player: admitted.player,
                                server,
                                mode: route.routing.server_config.proxy_mode,
                                backend,
                                lifecycle: admitted.lifecycle,
                                commands: admitted.commands,
                                session_token: admitted.session_token,
                                shutdown: self.shutdown.clone(),
                            }));
                        }
                        Err(error) => {
                            tracing::warn!(server = %server, error = %error, "backend connection failed");
                            Kick::failed(server, error, false)
                        }
                    }
                }
                Err(unavailable) => {
                    tracing::info!(
                        server = %server,
                        reason = ?unavailable,
                        "the server manager could not start the server"
                    );
                    unavailable.into_kick(server)
                }
            };
            let reason = match self.fire_kicked(&admitted, &kick).await {
                KickedFromServerResult::RedirectTo(next) if redirects < MAX_KICK_REDIRECTS => {
                    redirects += 1;
                    target = next;
                    cause = ConnectCause::KickRedirect;
                    continue;
                }
                KickedFromServerResult::RedirectTo(next) => {
                    tracing::warn!(
                        server = %kick.server,
                        target = %next,
                        "giving up after {MAX_KICK_REDIRECTS} kick redirects"
                    );
                    None
                }
                KickedFromServerResult::DisconnectPlayer { reason } => reason,
                KickedFromServerResult::Notify { message } => Some(message),
                KickedFromServerResult::SendToLimbo { .. } => {
                    tracing::warn!(
                        server = %kick.server,
                        "a plugin sent a forwarded connection to limbo after a kick, which needs the offline or client_only proxy mode; disconnecting the player"
                    );
                    None
                }
                _ => None,
            };
            let shown = reason.clone().or_else(|| kick.reason()).unwrap_or_else(|| {
                Component::text(route.routing.server_config.effective_disconnect_message())
            });
            self.kick(ctx, &shown).await;
            let ended = match kick.cause {
                KickCause::Unreachable { .. } => DisconnectCause::Error,
                _ => DisconnectCause::BackendClosed { reason },
            };
            admitted.lifecycle.end(ended).await;
            return Ok(None);
        }
    }

    fn resolve(
        &self,
        ctx: &mut ConnectionContext,
        server: &ServerId,
        origin: &Route,
    ) -> Result<Route, &'static str> {
        if server.as_str() == origin.routing.config_id {
            return Ok(origin.clone());
        }
        let services = self.services;
        let Some((server_config, load_balancer)) = services
            .domain_router
            .find_route_by_server_id(server.as_str())
        else {
            tracing::warn!(
                from = %origin.routing.config_id,
                to = %server,
                "plugin redirected a forwarded connection to an unknown server"
            );
            return Err(UNKNOWN_SERVER);
        };
        if matches!(
            server_config.proxy_mode,
            ProxyMode::Offline | ProxyMode::ClientOnly
        ) {
            tracing::warn!(
                from = %origin.routing.config_id,
                to = %server,
                mode = ?server_config.proxy_mode,
                "plugin redirected a forwarded connection to a server where the proxy runs the login, which a forwarded connection cannot do; disconnecting the player"
            );
            return Err(PROXY_LOGIN_SERVER);
        }
        let routing = RoutingData {
            server_config,
            config_id: server.to_string(),
            load_balancer,
        };
        let addresses = self.select(ctx, &routing);
        Ok(Route { routing, addresses })
    }

    fn select(&self, ctx: &mut ConnectionContext, routing: &RoutingData) -> Vec<ServerAddress> {
        let services = self.services;
        let addresses = select_backend_addresses(
            &routing.server_config,
            routing.load_balancer.as_ref(),
            services.pending_backends.as_ref(),
            services.backend_health.as_ref(),
        )
        .to_vec();
        if let Some(first) = addresses.first() {
            ctx.extensions
                .insert(services.pending_backends.reserve(first));
        }
        addresses
    }

    async fn open_backend(
        &self,
        ctx: &ConnectionContext,
        admitted: &Admitted,
        route: &Route,
        opening: &Opening<'_>,
    ) -> Result<BackendConnection, CoreError> {
        let config = &route.routing.server_config;
        let mut backend = self
            .connector
            .connect(
                &route.routing.config_id,
                &route.addresses,
                config.timeouts.as_ref().map(|t| t.connect),
                config.send_proxy_protocol,
                &ctx.connection_info(),
            )
            .await?;
        admitted
            .player
            .set_connected_address(Some(backend.server_address().clone()));
        let sent = match opening {
            Opening::Modern(handshake) => {
                let profile = admitted.player.profile();
                let data = ForwardingData {
                    real_ip: ctx.client_ip,
                    uuid: profile.uuid,
                    username: profile.username.clone(),
                    properties: profile.properties.clone(),
                    protocol_version: handshake.protocol_version,
                    chat_session: None,
                };
                send_initial_packets(
                    self.services,
                    backend.stream_mut(),
                    handshake,
                    config,
                    &data,
                )
                .await
            }
            Opening::Legacy(raw) => send_raw(backend.stream_mut(), raw).await,
        };
        if let Err(e) = sent {
            admitted.player.set_connected_address(None);
            return Err(e);
        }
        Ok(backend)
    }

    async fn fire_kicked(&self, admitted: &Admitted, kick: &Kick) -> KickedFromServerResult {
        let event = KickedFromServerEvent::new(
            Arc::clone(&admitted.player) as Arc<dyn Player>,
            kick.server.clone(),
            kick.reason(),
            kick.cause.clone(),
            kick.during_connect,
            admitted.player.current_server(),
            KickedFromServerResult::DisconnectPlayer { reason: None },
        );
        self.services.event_bus.fire(event).await.result().clone()
    }

    async fn interrupted(
        &self,
        ctx: &mut ConnectionContext,
        admitted: &mut Admitted,
    ) -> Option<DisconnectCause> {
        if !admitted.session_token.is_cancelled() {
            return None;
        }
        let reason = queued_kick(&mut admitted.commands).or_else(|| {
            self.shutdown
                .is_cancelled()
                .then(|| Component::text(SHUTDOWN_REASON))
        });
        if let Some(reason) = &reason {
            self.kick(ctx, reason).await;
        }
        Some(cancelled_cause(self.shutdown, reason))
    }

    async fn refuse_limbo(&self, ctx: &mut ConnectionContext, admitted: Admitted, event: &str) {
        tracing::warn!(
            player = %admitted.player.profile().username,
            event,
            "a plugin sent a forwarded connection to limbo, which needs the offline or client_only proxy mode; disconnecting the player"
        );
        let reason = Component::text(LIMBO_UNAVAILABLE);
        self.kick(ctx, &reason).await;
        admitted
            .lifecycle
            .end(DisconnectCause::Kicked {
                reason: Some(reason),
            })
            .await;
    }

    async fn kick(&self, ctx: &mut ConnectionContext, reason: &Component) {
        let sent = match self.wire {
            Wire::Modern(version) => {
                super::helpers::send_login_disconnect(
                    ctx.stream_mut(),
                    reason,
                    version,
                    &self.services.packet_registry,
                )
                .await
            }
            Wire::Legacy => super::helpers::send_legacy_kick(ctx.stream_mut(), reason).await,
        };
        if let Err(e) = sent {
            tracing::debug!(error = %e, "could not send the disconnect to the client");
        }
    }
}

fn queued_kick(commands: &mut mpsc::Receiver<PlayerCommand>) -> Option<Component> {
    while let Ok(command) = commands.try_recv() {
        if let PlayerCommand::Kick(reason) = command {
            return Some(reason);
        }
    }
    None
}

fn cancelled_cause(shutdown: &CancellationToken, reason: Option<Component>) -> DisconnectCause {
    if shutdown.is_cancelled() {
        DisconnectCause::Shutdown
    } else {
        DisconnectCause::Kicked { reason }
    }
}

async fn send_raw(backend: &mut TcpStream, raw: &[u8]) -> Result<(), CoreError> {
    backend.write_all(raw).await?;
    backend.flush().await?;
    Ok(())
}

async fn send_initial_packets(
    services: &ProxyServices,
    backend: &mut TcpStream,
    handshake: &HandshakeData,
    server_config: &ServerConfig,
    data: &ForwardingData,
) -> Result<(), CoreError> {
    let handler = services.resolve_forwarding_handler(server_config);

    if matches!(handler, ForwardingHandler::Velocity(_)) {
        tracing::warn!(
            "Velocity forwarding is configured for server '{}' in passthrough mode. \
             Velocity requires packet parsing and cannot work with passthrough. \
             Falling back to BungeeCord legacy forwarding.",
            server_config.effective_id()
        );
        let fallback =
            ForwardingHandler::Legacy(crate::forwarding::legacy::LegacyForwardingHandler);
        return send_with_forwarding(backend, handshake, server_config, data, &fallback).await;
    }

    if handler.modifies_handshake() {
        return send_with_forwarding(backend, handshake, server_config, data, &handler).await;
    }

    match &server_config.domain_rewrite {
        DomainRewrite::Explicit(new_domain) => {
            send_with_rewritten_handshake(backend, handshake, new_domain).await?;
        }
        DomainRewrite::FromBackend => {
            if let Some(addr) = server_config.addresses.first() {
                send_with_rewritten_handshake(backend, handshake, &addr.address.host).await?;
            } else {
                send_packets(backend, &handshake.raw_packets).await?;
            }
        }
        _ => send_packets(backend, &handshake.raw_packets).await?,
    }

    backend.flush().await?;
    Ok(())
}

async fn send_packets(
    backend: &mut TcpStream,
    packets: &[bytes::BytesMut],
) -> Result<(), CoreError> {
    for raw in packets {
        backend.write_all(raw).await?;
    }
    Ok(())
}

async fn send_with_forwarding(
    backend: &mut TcpStream,
    handshake: &HandshakeData,
    server_config: &ServerConfig,
    data: &ForwardingData,
    handler: &ForwardingHandler,
) -> Result<(), CoreError> {
    let mut modified = build_handshake_for_backend(handshake, server_config);
    handler.apply_handshake(&mut modified, data);

    let mut payload = Vec::new();
    modified.encode(&mut payload, handshake.protocol_version)?;

    let mut encoder = PacketEncoder::new();
    encoder.append_raw(0x00, &payload)?;
    backend.write_all(&encoder.take()).await?;
    send_packets(backend, handshake.raw_packets.get(1..).unwrap_or_default()).await?;
    backend.flush().await?;
    Ok(())
}

async fn send_with_rewritten_handshake(
    backend: &mut TcpStream,
    handshake: &HandshakeData,
    new_domain: &str,
) -> Result<(), CoreError> {
    let encoded = crate::util::domain_rewrite::encode_handshake_with_domain(handshake, new_domain)?;
    backend.write_all(&encoded).await?;
    send_packets(backend, handshake.raw_packets.get(1..).unwrap_or_default()).await
}
