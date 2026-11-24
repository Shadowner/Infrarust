use std::io::ErrorKind;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use infrarust_api::events::handshake::RejectReason;
use infrarust_api::filter::TransportContext;
use infrarust_api::handoff::Handoff;
use infrarust_api::types::Component;
use infrarust_config::{ProxyMode, UnknownDomainBehavior};
use infrarust_protocol::ProtocolError;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{Listener, PendingConnection, TransportError};

use super::ProxyServer;
use crate::error::CoreError;
use crate::handoff::raw_claim::{self, RawClaim, RawClaimLimits};
use crate::handoff::{HandoffError, HandoffLevel};
use crate::hot_restart::InFlight;
use crate::pipeline::admission::{self, Admission};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::MiddlewareResult;
use crate::pipeline::types::{ConnectionIntent, LegacyHello};

enum ConnectionError {
    BeforeLogin(CoreError),
    Session(CoreError),
}

impl From<CoreError> for ConnectionError {
    fn from(error: CoreError) -> Self {
        Self::BeforeLogin(error)
    }
}

fn client_gave_up(error: &CoreError) -> bool {
    let io = match error {
        CoreError::ConnectionClosed | CoreError::ClientLeft | CoreError::Timeout(_) => return true,
        CoreError::Io(io) | CoreError::Protocol(ProtocolError::Io(io)) => io,
        _ => return false,
    };
    matches!(
        io.kind(),
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::UnexpectedEof
            | ErrorKind::TimedOut
    )
}

impl ProxyServer {
    pub async fn serve(self: Arc<Self>, listener: Listener) -> Result<(), CoreError> {
        let config = &self.services.config;

        if let Some(ref sm) = self.services.server_manager {
            sm.initial_health_check().await;
            let player_counter: Arc<dyn infrarust_server_manager::PlayerCounter> =
                Arc::clone(&self.services.connection_registry) as _;
            let _monitoring_handles = sm.start_monitoring(player_counter, self.background.clone());
            tracing::info!("server manager monitoring started");
        }

        let _purge_handle = self
            .services
            .ban_manager
            .start_purge_task(config.ban.purge_interval, self.background.clone());

        let _rate_limit_purge_handle = self.rate_limiter.start_purge_task(self.background.clone());

        let _prober_handle = Arc::new(super::health_prober::ActiveHealthProber::new(
            Arc::clone(&self.services.domain_router),
            Arc::clone(&self.backend_health),
            Arc::clone(&self.services.packet_registry),
            config,
        ))
        .spawn(self.background.clone());

        loop {
            let pending = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => {
                    tracing::info!("proxy server shutting down");
                    break;
                }
                result = listener.accept() => {
                    match result {
                        Ok(conn) => conn,
                        Err(e) => {
                            tracing::warn!(error = %e, "accept error");
                            continue;
                        }
                    }
                }
            };

            let in_flight = self.services.relays.in_flight_guard();
            let pipeline = self.services.relays.pipeline().clone();
            let peer = pending.peer_addr();
            tracing::debug!(peer = %peer, "new connection");

            let server = Arc::clone(&self);
            self.connections.spawn(async move {
                match server.handle_connection(pending, pipeline, in_flight).await {
                    Ok(()) => {}
                    Err(ConnectionError::BeforeLogin(e)) if client_gave_up(&e) => {
                        tracing::debug!(peer = %peer, error = %e, "client left before login");
                    }
                    Err(ConnectionError::BeforeLogin(e) | ConnectionError::Session(e)) => {
                        tracing::warn!(peer = %peer, error = %e, "connection error");
                    }
                }
            });
        }

        Ok(())
    }

    async fn handle_connection(
        &self,
        pending: PendingConnection,
        shutdown: CancellationToken,
        in_flight: InFlight,
    ) -> Result<(), ConnectionError> {
        let Some(ctx) = self.accept(pending, in_flight, &shutdown).await? else {
            return Ok(());
        };
        let Some(mut ctx) = self.claim_raw(ctx, &shutdown).await else {
            return Ok(());
        };
        if !self.passes_common_pipeline(&mut ctx, &shutdown).await? {
            return Ok(());
        }

        let intent = ctx.require_handshake()?.intent;
        let Some(mut ctx) = self.screen(ctx, intent, &shutdown).await else {
            return Ok(());
        };

        if let Some(hello) = ctx.legacy_hello.clone() {
            return match hello {
                LegacyHello::Ping { request, raw } => self
                    .legacy_handler
                    .handle_ping(&mut ctx, &request, &raw)
                    .await
                    .map_err(ConnectionError::BeforeLogin),
                LegacyHello::Login { request, raw } => self
                    .legacy_handler
                    .handle_login(&mut ctx, &request, &raw)
                    .await
                    .map_err(ConnectionError::Session),
            };
        }

        match intent {
            ConnectionIntent::Status => Ok(self.answer_status(&mut ctx, &shutdown).await?),
            ConnectionIntent::Login | ConnectionIntent::Transfer => {
                let Some(ctx) = self.passes_login_pipeline(ctx, &shutdown).await? else {
                    return Ok(());
                };
                self.run_session(ctx, &shutdown).await
            }
        }
    }

    async fn accept(
        &self,
        pending: PendingConnection,
        in_flight: InFlight,
        shutdown: &CancellationToken,
    ) -> Result<Option<ConnectionContext>, CoreError> {
        let accepted = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(None),
            accepted = pending.resolve() => match accepted {
                Ok(accepted) => accepted,
                Err(TransportError::IpLimitReached { ip }) => {
                    tracing::debug!(%ip, "closing connection over the per-IP limit");
                    return Ok(None);
                }
                Err(TransportError::UntrustedProxyPeer { peer }) => {
                    tracing::debug!(%peer, "closing connection from a peer outside trusted_proxies");
                    return Ok(None);
                }
                Err(e) => return Err(CoreError::from(e)),
            },
        };
        let mut ctx = ConnectionContext::from_accepted(accepted);
        ctx.hold_in_flight(in_flight);
        if !self.open_transport(&mut ctx, shutdown).await {
            return Ok(None);
        }
        Ok(Some(ctx))
    }

    async fn claim_raw(
        &self,
        mut ctx: ConnectionContext,
        shutdown: &CancellationToken,
    ) -> Option<ConnectionContext> {
        if self.services.connection_claims.is_empty() {
            return Some(ctx);
        }
        match raw_claim::claim_raw(
            &self.services,
            &mut ctx,
            RawClaimLimits::default(),
            shutdown,
        )
        .await
        {
            RawClaim::Minecraft => Some(ctx),
            RawClaim::Closed => None,
            RawClaim::Handoff(handoff) => {
                self.hand_off(ctx, handoff, HandoffLevel::Raw, shutdown)
                    .await;
                None
            }
        }
    }

    async fn passes_common_pipeline(
        &self,
        ctx: &mut ConnectionContext,
        shutdown: &CancellationToken,
    ) -> Result<bool, CoreError> {
        let common = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(false),
            result = self.common_pipeline.execute(ctx) => result?,
        };

        match common {
            MiddlewareResult::Continue => Ok(true),
            MiddlewareResult::Drop(reason) => {
                admission::reject_connection(&self.services.event_bus, ctx, reason);
                Ok(false)
            }
            MiddlewareResult::Claimed(handoff) => {
                tracing::error!(
                    target = %handoff.target,
                    "a login claim reached the common pipeline, closing the connection"
                );
                Ok(false)
            }
            MiddlewareResult::Reject { reason, message } => {
                let intent = ctx.handshake.as_ref().map(|h| h.intent);
                let unknown_domain = reason == RejectReason::UnknownDomain;
                let dropped =
                    unknown_domain && self.unknown_domain_behavior == UnknownDomainBehavior::Drop;
                if !dropped && unknown_domain && intent == Some(ConnectionIntent::Status) {
                    return Ok(true);
                }
                admission::reject_connection(&self.services.event_bus, ctx, reason);
                match intent {
                    Some(ConnectionIntent::Login | ConnectionIntent::Transfer) if !dropped => {
                        self.send_kick(ctx, &Component::text(message)).await.ok();
                    }
                    _ => tracing::debug!("closing rejected connection: {message}"),
                }
                Ok(false)
            }
            MiddlewareResult::Kick { reason, message } => {
                admission::reject_connection(&self.services.event_bus, ctx, reason);
                self.send_kick(ctx, &message).await.ok();
                Ok(false)
            }
        }
    }

    async fn screen(
        &self,
        mut ctx: ConnectionContext,
        intent: ConnectionIntent,
        shutdown: &CancellationToken,
    ) -> Option<ConnectionContext> {
        let screened = tokio::select! {
            biased;
            () = shutdown.cancelled() => return None,
            screened = admission::screen_connection(&self.services.event_bus, &ctx) => screened,
        };
        match screened {
            Admission::Admitted => Some(ctx),
            Admission::Denied(reason) => {
                if intent != ConnectionIntent::Status {
                    self.send_kick(&mut ctx, &reason).await.ok();
                }
                None
            }
            Admission::Dropped => None,
            Admission::Handoff(_) if ctx.legacy_hello.is_some() => {
                tracing::debug!("a client older than 1.7 cannot be handed off, closing it");
                None
            }
            Admission::Handoff(handoff) => {
                let refused = tokio::select! {
                    biased;
                    () = shutdown.cancelled() => return None,
                    refused = admission::refuse_handoff(&self.services.ban_manager, &ctx) => refused,
                };
                if let Some(refusal) = refused {
                    admission::reject_connection(&self.services.event_bus, &ctx, refusal.reason);
                    if intent != ConnectionIntent::Status {
                        self.send_kick(&mut ctx, &refusal.message).await.ok();
                    }
                    return None;
                }
                self.hand_off(ctx, handoff, HandoffLevel::Handshake, shutdown)
                    .await;
                None
            }
        }
    }

    async fn passes_login_pipeline(
        &self,
        mut ctx: ConnectionContext,
        shutdown: &CancellationToken,
    ) -> Result<Option<ConnectionContext>, CoreError> {
        let login = tokio::select! {
            biased;
            () = shutdown.cancelled() => None,
            result = self.login_pipeline.execute(&mut ctx) => Some(result?),
        };
        let Some(login) = login else {
            let reason = Component::text(self.services.relays.notice().reason());
            self.send_kick(&mut ctx, &reason).await.ok();
            return Ok(None);
        };

        match login {
            MiddlewareResult::Continue => return Ok(Some(ctx)),
            MiddlewareResult::Claimed(handoff) => {
                self.hand_off(ctx, handoff, HandoffLevel::Login, shutdown)
                    .await;
            }
            MiddlewareResult::Drop(reason) => {
                admission::reject_connection(&self.services.event_bus, &ctx, reason);
            }
            MiddlewareResult::Reject { reason, message } => {
                admission::reject_connection(&self.services.event_bus, &ctx, reason);
                self.send_kick(&mut ctx, &Component::text(message))
                    .await
                    .ok();
            }
            MiddlewareResult::Kick { reason, message } => {
                admission::reject_connection(&self.services.event_bus, &ctx, reason);
                self.send_kick(&mut ctx, &message).await.ok();
            }
        }
        Ok(None)
    }

    async fn run_session(
        &self,
        mut ctx: ConnectionContext,
        shutdown: &CancellationToken,
    ) -> Result<(), ConnectionError> {
        let proxy_mode = ctx.require_routing()?.server_config.proxy_mode;

        let span = ctx.span.take().unwrap_or_else(tracing::Span::none);

        let session = shutdown.child_token();
        match proxy_mode {
            ProxyMode::Offline => {
                self.offline_handler
                    .handle(ctx, session)
                    .instrument(span)
                    .await
            }
            ProxyMode::ClientOnly => {
                self.client_only_handler
                    .handle(ctx, session)
                    .instrument(span)
                    .await
            }
            ProxyMode::Full => {
                tracing::error!(
                    server = %ctx
                        .require_routing()?
                        .server_config
                        .effective_id(),
                    "server configured with the reserved proxy_mode = \"full\", \
                     which configuration validation rejects"
                );
                return Err(CoreError::InvalidState(
                    "reserved proxy_mode = \"full\" reached the session handler",
                )
                .into());
            }
            _ => {
                self.passthrough_handler
                    .handle(ctx, session)
                    .instrument(span)
                    .await
            }
        }
        .map_err(ConnectionError::Session)
    }

    async fn hand_off(
        &self,
        mut ctx: ConnectionContext,
        handoff: Handoff,
        level: HandoffLevel,
        shutdown: &CancellationToken,
    ) {
        ctx.settle();
        let client = ctx.client_addr();
        let target = handoff.target.clone();
        match self
            .handoff
            .relay(ctx, handoff, level, shutdown.child_token())
            .await
        {
            Ok(_) | Err(HandoffError::Shutdown) => {}
            Err(error) => tracing::warn!(
                client = %client,
                target = %target,
                level = level.as_str(),
                error = %error,
                "connection handoff failed, closing the connection"
            ),
        }
    }

    async fn open_transport(
        &self,
        ctx: &mut ConnectionContext,
        shutdown: &CancellationToken,
    ) -> bool {
        let chain = self.services.transport_filter_registry.chain();
        if chain.is_empty() {
            return true;
        }
        let timeout = self.services.config.events.transport_filter_timeout;
        let opened = tokio::select! {
            biased;
            () = shutdown.cancelled() => return false,
            opened = chain.open(transport_context(ctx), timeout) => opened,
        };
        match opened {
            Ok(session) => {
                ctx.attach_transport(session);
                true
            }
            Err(rejection) => {
                rejection.log(ctx.client_addr());
                admission::reject(
                    &self.services.event_bus,
                    ctx.client_addr(),
                    None,
                    rejection.reason(),
                );
                false
            }
        }
    }

    async fn answer_status(
        &self,
        ctx: &mut ConnectionContext,
        shutdown: &CancellationToken,
    ) -> Result<(), CoreError> {
        let status = self
            .status_handler
            .handle(ctx, &self.services.connection_registry);
        tokio::select! {
            biased;
            () = shutdown.cancelled() => Ok(()),
            result = status => result,
        }
    }

    async fn send_kick(
        &self,
        ctx: &mut ConnectionContext,
        reason: &Component,
    ) -> Result<(), CoreError> {
        if ctx.legacy_hello.is_some() {
            return crate::handler::helpers::send_legacy_kick(ctx.stream_mut(), reason).await;
        }
        let version = ctx
            .handshake
            .as_ref()
            .map_or(ProtocolVersion::CURRENT, |h| h.protocol_version);

        crate::handler::helpers::send_login_disconnect(
            ctx.stream_mut(),
            reason,
            version,
            &self.services.packet_registry,
        )
        .await
    }
}

fn transport_context(ctx: &ConnectionContext) -> TransportContext {
    TransportContext::new(
        ctx.peer_addr(),
        ctx.local_addr(),
        ctx.real_ip(),
        ctx.connected_at().into_std(),
        ctx.connection_id,
    )
}
