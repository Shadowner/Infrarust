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
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};
use tokio_util::sync::CancellationToken;

use crate::error::CoreError;
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::{Login, connect_backend};
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::Kick;
use crate::session::server_join::{ServerJoin, pre_connect};
use crate::session::wake::{WakeFailure, wake};

use config_phase::PhaseError;

const SWITCH_PHASE_TIMEOUT: Duration = Duration::from_secs(30);

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

pub(crate) async fn perform_switch(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    current_server: &ServerId,
    target: SwitchTarget,
) -> Result<SwitchResult, CoreError> {
    let client = &mut io.client;
    let services = ctx.services;
    let session = &ctx.session;
    let version = ctx.version();
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

    if let SwitchTarget::Unapproved { cause, .. } = &target
        && !validation::switch_in_scope(*cause, &current_config, &server_config)
    {
        return Err(CoreError::Rejected(format!(
            "server '{}' is outside the scope of '{}'",
            requested.as_str(),
            current_server.as_str()
        )));
    }

    let effective_target = match target {
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
                        .resolve_handlers(&handler_names)?;
                    let ctx = LimboEntryContext::PluginRedirect {
                        from_server: Some(current_server.clone()),
                    };
                    return Ok(SwitchResult::Limbo(handlers, ctx));
                }
                _ => server,
            };
            if matches!(cause, ConnectCause::Switch | ConnectCause::PluginMessage)
                && effective == *current_server
            {
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

    if effective_target != *current_server
        && !validation::admits_session(session.kind(), &server_config)
    {
        tracing::info!(
            username = %session.game_profile().username,
            target = %effective_target,
            "refusing to switch an unverified player to a server that requires verification"
        );
        return Ok(SwitchResult::Denied(Component::text(
            validation::UNVERIFIED_REFUSED,
        )));
    }

    match wake(
        services,
        &server_config,
        session.shutdown_token(),
        client.stream(),
    )
    .await
    {
        Ok(_) => {}
        Err(WakeFailure::Unavailable(unavailable)) => {
            return Ok(SwitchResult::Failed(
                unavailable.into_kick(effective_target),
            ));
        }
        Err(WakeFailure::ClientLeft) => return Err(CoreError::ClientLeft),
    }

    // Same strategy + unhealthy-last ordering as the login pipeline.
    let addresses = crate::loadbalancer::select_backend_addresses(
        &server_config,
        load_balancer.as_ref(),
        services.pending_backends.as_ref(),
        services.backend_health.as_ref(),
    );

    let mut new_backend = match connect_backend(
        ctx,
        effective_target.as_str(),
        &server_config,
        &addresses,
        Login::Proxied,
    )
    .await
    {
        Ok(backend) => backend,
        Err(e) => {
            return Ok(SwitchResult::Failed(Kick::failed(
                effective_target,
                e,
                false,
            )));
        }
    };

    let mut join = ServerJoin::new(session, effective_target.clone());
    join.connected(&services.event_bus).await;

    let mut stranded = client.state() != ConnectionState::Play;
    let session_token = session.shutdown_token();
    let config_phase = version.no_less_than(ProtocolVersion::V1_20_2);
    if config_phase
        && let Err(e) =
            replay::replay(&mut new_backend, session, services, &server_config, version).await
    {
        return Ok(SwitchResult::Failed(Kick::failed(
            effective_target,
            e,
            stranded,
        )));
    }
    let first_frame = if config_phase {
        let chain = FrameChain::new(ctx, &effective_target);
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
        Err(PhaseError::Backend(e)) => {
            return Ok(SwitchResult::Failed(Kick::failed(
                effective_target,
                e,
                stranded,
            )));
        }
    };
    if !config_phase
        && let Err(e) =
            replay::replay(&mut new_backend, session, services, &server_config, version).await
    {
        return Ok(SwitchResult::Failed(Kick::failed(
            effective_target,
            e,
            stranded,
        )));
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
        new = %effective_target,
        "server switch complete"
    );

    Ok(SwitchResult::Backend(SwitchSuccess {
        new_backend,
        new_server_id: effective_target,
    }))
}
