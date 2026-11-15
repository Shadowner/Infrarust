//! Server switch orchestrator.
//!
//! Handles transferring a player from one backend server to another without
//! disconnecting them. The mechanism is version-dependent — see `switch_packets`
//! and `config_phase` submodules for details.

mod config_phase;
mod replay;
mod switch_packets;
pub(crate) mod validation;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{ConnectCause, ServerPreConnectResult};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::types::{Component, ServerId};
use infrarust_config::ServerConfig;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;
use crate::loadbalancer::LoadBalancer;
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::{Login, connect_backend};
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::Kick;
use crate::session::server_join::{ServerJoin, pre_connect};
use crate::session::session_loop::LimboEntry;
use crate::session::wake::{WakeFailure, wake};

use config_phase::PhaseError;

const SWITCH_PHASE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) enum SwitchResult {
    Backend(BackendBridge, ServerId),
    Limbo(LimboEntry),
    Denied(Component),
    Unchanged,
    Failed(Kick),
    Error(CoreError),
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

async fn guarded(
    token: &CancellationToken,
    phase: impl Future<Output = Result<PacketFrame, PhaseError>>,
) -> Result<PacketFrame, PhaseError> {
    tokio::select! {
        () = token.cancelled() => Err(PhaseError::Client(CoreError::ConnectionClosed)),
        phase = tokio::time::timeout(SWITCH_PHASE_TIMEOUT, phase) => phase.unwrap_or_else(|_| {
            Err(PhaseError::Backend(CoreError::Timeout(
                "server switch timed out waiting for the backend".into(),
            )))
        }),
    }
}

struct Destination {
    server: ServerId,
    config: Arc<ServerConfig>,
    load_balancer: Arc<dyn LoadBalancer>,
}

enum Approval {
    Go(Destination),
    Settled(SwitchResult),
}

pub(crate) async fn perform_switch(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    current_server: &ServerId,
    target: SwitchTarget,
) -> Result<SwitchResult, CoreError> {
    let destination = match approve(ctx, current_server, target).await? {
        Approval::Go(destination) => destination,
        Approval::Settled(result) => return Ok(result),
    };
    let client = &mut io.client;
    let services = ctx.services;

    match wake(
        services,
        &destination.config,
        ctx.session.shutdown_token(),
        client.stream(),
    )
    .await
    {
        Ok(_) => {}
        Err(WakeFailure::Unavailable(unavailable)) => {
            return Ok(SwitchResult::Failed(
                unavailable.into_kick(destination.server),
            ));
        }
        Err(WakeFailure::ClientLeft) => return Err(CoreError::ClientLeft),
    }

    let addresses = crate::loadbalancer::select_backend_addresses(
        &destination.config,
        destination.load_balancer.as_ref(),
        services.pending_backends.as_ref(),
        services.backend_health.as_ref(),
    );

    let new_backend = match connect_backend(
        ctx,
        destination.server.as_str(),
        &destination.config,
        &addresses,
        Login::Proxied,
    )
    .await
    {
        Ok(backend) => backend,
        Err(e) => {
            return Ok(SwitchResult::Failed(Kick::failed(
                destination.server,
                e,
                false,
            )));
        }
    };

    hand_over(ctx, client, new_backend, current_server, &destination).await
}

async fn approve(
    ctx: &SessionContext<'_>,
    current_server: &ServerId,
    target: SwitchTarget,
) -> Result<Approval, CoreError> {
    let services = ctx.services;
    let session = &ctx.session;
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

    if let Some((current, target)) = validation::different_networks(&current_config, &server_config)
    {
        return Err(CoreError::Rejected(format!(
            "servers are in different networks: '{current}' vs '{target}'"
        )));
    }

    if let SwitchTarget::Unapproved { cause, .. } = &target
        && !validation::switch_in_scope(*cause, &current_config, &server_config)
    {
        return Err(CoreError::Rejected(format!(
            "server '{}' is outside the scope of '{}'",
            requested.as_str(),
            current_server.as_str()
        )));
    }

    let server = match target {
        SwitchTarget::Approved(server) => server,
        SwitchTarget::Unapproved { server, cause } => {
            let pre_connect =
                pre_connect(&services.event_bus, session, server.clone(), cause).await;
            let effective = match pre_connect.result() {
                ServerPreConnectResult::Allowed => server,
                ServerPreConnectResult::Redirect(redirect) => {
                    tracing::info!(
                        original = %server,
                        redirect = %redirect,
                        "server switch redirected by event"
                    );
                    redirect.clone()
                }
                ServerPreConnectResult::Denied { reason } => {
                    return Ok(Approval::Settled(SwitchResult::Denied(reason.clone())));
                }
                ServerPreConnectResult::SendToLimbo { limbo_handlers } => {
                    tracing::info!("server switch redirected to limbo by event");
                    let handler_names = if limbo_handlers.is_empty() {
                        &server_config.limbo_handlers
                    } else {
                        limbo_handlers
                    };
                    let handlers = services
                        .limbo_handler_registry
                        .resolve_handlers(handler_names)?;
                    return Ok(Approval::Settled(SwitchResult::Limbo(LimboEntry {
                        handlers,
                        context: LimboEntryContext::PluginRedirect {
                            from_server: Some(current_server.clone()),
                        },
                    })));
                }
                _ => server,
            };
            if matches!(cause, ConnectCause::Switch | ConnectCause::PluginMessage)
                && effective == *current_server
            {
                return Ok(Approval::Settled(SwitchResult::Unchanged));
            }
            effective
        }
    };

    let (config, load_balancer) = if server == requested {
        (server_config, load_balancer)
    } else {
        services
            .domain_router
            .find_route_by_server_id(server.as_str())
            .ok_or_else(|| {
                CoreError::Rejected(format!("unknown redirect server: {}", server.as_str()))
            })?
    };

    if server != *current_server && !validation::admits_session(session.kind(), &config) {
        tracing::info!(
            username = %session.game_profile().username,
            target = %server,
            "refusing to switch an unverified player to a server that requires verification"
        );
        return Ok(Approval::Settled(SwitchResult::Denied(Component::text(
            validation::UNVERIFIED_REFUSED,
        ))));
    }

    Ok(Approval::Go(Destination {
        server,
        config,
        load_balancer,
    }))
}

async fn hand_over(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    mut new_backend: BackendBridge,
    current_server: &ServerId,
    destination: &Destination,
) -> Result<SwitchResult, CoreError> {
    let services = ctx.services;
    let session = &ctx.session;
    let version = ctx.version();
    let server = &destination.server;
    let failed = |error, stranded| {
        Ok(SwitchResult::Failed(Kick::failed(
            server.clone(),
            error,
            stranded,
        )))
    };

    let mut join = ServerJoin::new(session, server.clone());
    join.connected(&services.event_bus).await;

    let mut stranded = client.state() != ConnectionState::Play;
    let session_token = session.shutdown_token();
    let config_phase = version.no_less_than(ProtocolVersion::V1_20_2);
    if config_phase
        && let Err(e) = replay::replay(
            &mut new_backend,
            session,
            services,
            &destination.config,
            version,
        )
        .await
    {
        return failed(e, stranded);
    }
    let first_frame = if config_phase {
        let chain = FrameChain::new(ctx, server);
        guarded(
            session_token,
            config_phase::handle_config_phase_switch(
                ctx,
                client,
                &mut new_backend,
                &chain,
                &mut stranded,
            ),
        )
        .await
    } else {
        guarded(
            session_token,
            config_phase::first_play_frame(ctx, client, &mut new_backend),
        )
        .await
    };
    let join_game_frame = match first_frame {
        Ok(frame) => frame,
        Err(PhaseError::Client(e)) => return Err(e),
        Err(PhaseError::Backend(e)) => return failed(e, stranded),
    };
    if !config_phase
        && let Err(e) = replay::replay(
            &mut new_backend,
            session,
            services,
            &destination.config,
            version,
        )
        .await
    {
        return failed(e, stranded);
    }

    switch_packets::send_switch_packets(
        client,
        &join_game_frame,
        version,
        &services.packet_registry,
    )
    .await?;
    crate::session::presentation::restore_after_switch(
        client,
        session,
        &services.packet_registry,
        version,
    )
    .await?;

    join.joined(&services.event_bus).await;

    tracing::info!(
        previous = %current_server,
        new = %server,
        "server switch complete"
    );

    Ok(SwitchResult::Backend(new_backend, server.clone()))
}
