//! Server switch orchestrator.
//!
//! Handles transferring a player from one backend server to another without
//! disconnecting them. The mechanism is version-dependent — see `switch_packets`
//! and `config_phase` submodules for details.

mod config_phase;
mod replay;
mod switch_packets;
pub(crate) mod validation;

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::connection::{ConnectCause, ServerPreConnectResult};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::limbo::handler::LimboHandler;
use infrarust_api::types::{Component, ServerId};
use infrarust_protocol::packets::play::disconnect::CDisconnect;
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::session::backend_bridge::BackendBridge;
use crate::session::backend_login::{Login, connect_backend};
use crate::session::context::{SessionContext, SessionIo};
use crate::session::frame_chain::FrameChain;
use crate::session::kick::{BackendKick, Kick};
use crate::session::server_join::{ServerJoin, pre_connect};
use crate::session::wake::wake;

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
                        .resolve_handlers_lenient(&handler_names);
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

    if let Err(unavailable) = wake(services, &server_config, session.shutdown_token()).await {
        return Ok(SwitchResult::Failed(
            unavailable.into_kick(effective_target),
        ));
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
    let join_game_frame = if version.no_less_than(ProtocolVersion::V1_20_2) {
        if let Err(e) =
            replay::replay(&mut new_backend, session, services, &server_config, version).await
        {
            return Ok(SwitchResult::Failed(Kick::failed(
                effective_target,
                e,
                stranded,
            )));
        }
        let session_token = session.shutdown_token().clone();
        let chain = FrameChain::new(ctx, &effective_target);
        let config_phase = tokio::time::timeout(
            std::time::Duration::from_secs(SWITCH_CONFIG_PHASE_TIMEOUT_SECS),
            config_phase::handle_config_phase_switch(
                ctx,
                client,
                &mut new_backend,
                &chain,
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
            Ok(Some(frame)) => {
                if let Err(e) =
                    replay::replay(&mut new_backend, session, services, &server_config, version)
                        .await
                {
                    return Ok(SwitchResult::Failed(Kick::failed(
                        effective_target,
                        e,
                        stranded,
                    )));
                }
                frame
            }
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
