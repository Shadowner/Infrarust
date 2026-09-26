use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use infrarust_api::filter::TransportContext;
use infrarust_api::types::{Component, Extensions};
use infrarust_config::{ProxyMode, UnknownDomainBehavior};
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{Listener, PendingConnection};

use super::ProxyServer;
use crate::error::CoreError;
use crate::middleware::telemetry::ConnectionSpan;
use crate::pipeline::admission::{self, Admission};
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::middleware::MiddlewareResult;
use crate::pipeline::types::{
    ConnectionIntent, HandshakeData, LegacyDetected, RoutingData, UnknownDomain,
};
use crate::player::SHUTDOWN_REASON;

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

        let _prober_handle = Arc::new(crate::loadbalancer::ActiveHealthProber::new(
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

            let sessions = self.sessions.clone();
            let peer = pending.peer_addr();
            tracing::debug!(peer = %peer, "new connection");

            let server = Arc::clone(&self);
            self.connections.spawn(async move {
                if let Err(e) = server.handle_connection(pending, sessions).await {
                    tracing::warn!(peer = %peer, error = %e, "connection error");
                }
            });
        }

        Ok(())
    }

    async fn handle_connection(
        &self,
        pending: PendingConnection,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let accepted = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(()),
            accepted = pending.resolve() => accepted?,
        };
        let mut ctx = ConnectionContext::from_accepted(accepted);
        if !self.open_transport(&mut ctx, &shutdown).await {
            return Ok(());
        }

        let common = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(()),
            result = self.common_pipeline.execute(&mut ctx) => result?,
        };

        match common {
            MiddlewareResult::Continue => {}
            MiddlewareResult::ShortCircuit => {
                if ctx.extensions.contains::<LegacyDetected>() {
                    return self.legacy_handler.handle(&mut ctx).await;
                }
                admission::reject_refused(&self.services.event_bus, &ctx);
                return Ok(());
            }
            MiddlewareResult::Reject(msg) => {
                let is_status = ctx
                    .extensions
                    .get::<HandshakeData>()
                    .is_some_and(|h| h.intent == ConnectionIntent::Status);
                let answered = is_status
                    && ctx.extensions.contains::<UnknownDomain>()
                    && self.unknown_domain_behavior != UnknownDomainBehavior::Drop;
                if !answered {
                    admission::reject_refused(&self.services.event_bus, &ctx);
                    if self.unknown_domain_behavior == UnknownDomainBehavior::Drop {
                        tracing::debug!("dropping connection: {msg}");
                    } else if !is_status {
                        self.send_kick(&mut ctx, &Component::text(msg)).await.ok();
                    }
                    return Ok(());
                }
            }
            MiddlewareResult::Kick(reason) => {
                admission::reject_refused(&self.services.event_bus, &ctx);
                self.send_kick(&mut ctx, &reason).await.ok();
                return Ok(());
            }
        }

        let intent = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .intent;

        let screened = tokio::select! {
            biased;
            () = shutdown.cancelled() => return Ok(()),
            screened = admission::screen_connection(&self.services.event_bus, &ctx) => screened,
        };
        match screened {
            Admission::Admitted => {}
            Admission::Denied(reason) => {
                if intent != ConnectionIntent::Status {
                    self.send_kick(&mut ctx, &reason).await.ok();
                }
                return Ok(());
            }
            Admission::Dropped => return Ok(()),
        }

        match intent {
            ConnectionIntent::Status => self.answer_status(&mut ctx, &shutdown).await?,
            ConnectionIntent::Login | ConnectionIntent::Transfer => {
                let login = tokio::select! {
                    biased;
                    () = shutdown.cancelled() => None,
                    result = self.login_pipeline.execute(&mut ctx) => Some(result?),
                };
                let Some(login) = login else {
                    self.send_kick(&mut ctx, &Component::text(SHUTDOWN_REASON))
                        .await
                        .ok();
                    return Ok(());
                };

                match login {
                    MiddlewareResult::Continue => {}
                    MiddlewareResult::ShortCircuit => return Ok(()),
                    MiddlewareResult::Reject(msg) => {
                        admission::reject_refused(&self.services.event_bus, &ctx);
                        self.send_kick(&mut ctx, &Component::text(msg)).await.ok();
                        return Ok(());
                    }
                    MiddlewareResult::Kick(reason) => {
                        admission::reject_refused(&self.services.event_bus, &ctx);
                        self.send_kick(&mut ctx, &reason).await.ok();
                        return Ok(());
                    }
                }

                let proxy_mode = ctx
                    .require_extension::<RoutingData>("RoutingData")?
                    .server_config
                    .proxy_mode;

                let span = ctx
                    .extensions
                    .remove::<ConnectionSpan>()
                    .map_or_else(tracing::Span::none, |cs| cs.0);

                match proxy_mode {
                    ProxyMode::Offline => {
                        self.offline_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    ProxyMode::ClientOnly => {
                        self.client_only_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                    ProxyMode::Full => {
                        tracing::error!(
                            server = %ctx
                                .require_extension::<RoutingData>("RoutingData")?
                                .server_config
                                .effective_id(),
                            "server configured with the reserved proxy_mode = \"full\", \
                             which configuration validation rejects"
                        );
                        return Err(CoreError::InvalidState(
                            "reserved proxy_mode = \"full\" reached the session handler",
                        ));
                    }
                    _ => {
                        self.passthrough_handler
                            .handle(ctx, shutdown.child_token())
                            .instrument(span)
                            .await?;
                    }
                }
            }
        }

        Ok(())
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
        let version = ctx
            .extensions
            .get::<HandshakeData>()
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
    TransportContext {
        remote_addr: ctx.peer_addr,
        local_addr: ctx.local_addr,
        real_ip: ctx.real_ip,
        connection_time: ctx.connected_at.into_std(),
        connection_id: ctx.connection_id,
        extensions: Extensions::new(),
    }
}
