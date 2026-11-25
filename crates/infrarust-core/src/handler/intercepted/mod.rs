//! Unified handler for `ClientOnly` and `Offline` intercepted proxy modes.

pub(crate) mod auth;

use std::sync::Arc;

use infrarust_api::__private::session_task;
use infrarust_api::events::lifecycle::DisconnectCause;
use infrarust_api::player::Player;
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use infrarust_transport::BackendConnector;

use super::helpers::cancelled_cause;
use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::forwarding::domain_rewrite::as_login;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, RoutingData};
use crate::player::commands::CommandInbox;
use crate::player::{SessionKind, ShutdownKind};
use crate::server::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::login_admission::{Admitted, Arrival, LoginAdmission, admit};
use crate::session::proxy_loop::ProxyLoopOutcome;
use crate::session::resume::{self, Claim};

use crate::session::context::{SessionContext, SessionIo};
use crate::session::initial_connect::{InitialMode, LoginFacts, resolve_initial_mode};
use crate::session::session_loop::{ConnectionMode, run_session_loop};
use auth::{AuthStrategy, Authenticated};

pub struct InterceptedHandler {
    backend_connector: Arc<BackendConnector>,
    services: ProxyServices,
    auth_strategy: AuthStrategy,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl InterceptedHandler {
    pub fn client_only(
        backend_connector: Arc<BackendConnector>,
        services: ProxyServices,
        auth: Arc<MojangAuth>,
    ) -> Self {
        Self {
            backend_connector,
            services,
            auth_strategy: AuthStrategy::Mojang(auth),
            #[cfg(feature = "telemetry")]
            metrics: None,
        }
    }

    pub fn offline(
        backend_connector: Arc<BackendConnector>,
        services: ProxyServices,
        mojang_auth: Option<Arc<MojangAuth>>,
    ) -> Self {
        Self {
            backend_connector,
            services,
            auth_strategy: AuthStrategy::Offline {
                mojang: mojang_auth,
            },
            #[cfg(feature = "telemetry")]
            metrics: None,
        }
    }

    #[cfg(feature = "telemetry")]
    pub fn with_metrics(mut self, metrics: Arc<crate::telemetry::ProxyMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    #[tracing::instrument(name = "proxy.session", skip_all, fields(mode = self.auth_strategy.mode_label()))]
    pub async fn handle(
        &self,
        ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let player = PlayerId::new(ctx.connection_id);
        session_task::scope(Some(player), self.serve(ctx, shutdown)).await
    }

    async fn serve(
        &self,
        mut conn: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = conn.require_routing()?.clone();
        let mut handshake = conn.require_handshake()?.clone();
        let login_data = conn.login.clone();

        let version = handshake.protocol_version;
        let registry = &self.services.packet_registry;

        let mut client = ClientBridge::new(conn.take_stream(), conn.buffered_data.split(), version);
        client.set_cancel(shutdown.clone());

        let claim = resume::claim(
            &self.services,
            &mut client,
            &handshake,
            login_data.as_ref(),
            conn.client_ip(),
        )
        .await?;
        let resumed = match claim {
            Claim::Absent => None,
            Claim::Presented => {
                as_login(&mut handshake)?;
                None
            }
            Claim::Resumed(record) => {
                as_login(&mut handshake)?;
                Some(record)
            }
        };
        let recognised = match &resumed {
            Some(record) => {
                self.auth_strategy
                    .resume(
                        &mut client,
                        record,
                        &self.services,
                        conn.client_addr(),
                        &handshake.domain,
                    )
                    .await?
            }
            None => None,
        };
        let authenticated = match recognised {
            Some(authenticated) => authenticated,
            None => {
                self.auth_strategy
                    .authenticate(
                        &mut client,
                        login_data.as_ref(),
                        &self.services,
                        version,
                        conn.client_addr(),
                        &handshake.domain,
                    )
                    .await?
            }
        };
        let resumed_server = resumed
            .map(|record| record.server)
            .filter(|server| !server.is_empty())
            .map(ServerId::new);

        let (profile, online_mode) = match authenticated {
            Authenticated::Denied(reason) => {
                client.disconnect(&reason, registry).await.ok();
                return Ok(());
            }
            Authenticated::Player {
                profile,
                online_mode,
            } => (profile, online_mode),
        };

        let arrival = Arrival {
            profile,
            protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
            domain: handshake.domain.clone(),
            origin: ServerId::new(routing.config_id.clone()),
        };
        let kind = SessionKind::Intercepted { online_mode };
        let admitted = match admit(&self.services, &conn, &shutdown, arrival, kind).await {
            LoginAdmission::Admitted(admitted) => admitted,
            LoginAdmission::Refused(reason) => {
                client.disconnect(&reason, registry).await.ok();
                return Ok(());
            }
        };
        client.set_cancel(admitted.token.clone());

        let entry = Entry {
            routing,
            handshake,
            online_mode,
            resumed_server,
        };
        self.play(conn, &shutdown, client, admitted, entry).await
    }

    async fn play(
        &self,
        mut conn: ConnectionContext,
        shutdown: &CancellationToken,
        client: ClientBridge,
        admitted: Admitted,
        entry: Entry,
    ) -> Result<(), CoreError> {
        let Admitted {
            session: player,
            lifecycle,
            commands,
            token,
            login_link: _,
            rewritten,
        } = admitted;
        let Entry {
            routing,
            handshake,
            online_mode,
            resumed_server,
        } = entry;
        let registry = &self.services.packet_registry;
        let (client_codec, server_codec) = crate::filter::codec_chain::build_codec_chains(
            &self.services.codec_filter_registry,
            infrarust_api::types::ProtocolVersion::new(handshake.protocol_version.0),
            player.id().as_u64(),
            conn.peer_addr(),
            Some(conn.client_ip()),
        );
        let mut io = SessionIo {
            client,
            commands: CommandInbox::new(commands).with_session(Arc::clone(&player)),
            client_codec,
            server_codec,
        };
        let ctx = SessionContext {
            services: &self.services,
            backend_connector: &self.backend_connector,
            session: Arc::clone(&player),
            handshake,
            connection_info: conn.connection_info().clone(),
            token,
            resumed_server,
        };

        if let Some(cause) = ended_before_start(&ctx, &mut io, shutdown).await {
            io.close();
            lifecycle.end(cause).await;
            return Ok(());
        }

        let backend_targets = conn.backend_targets.clone();
        let mut pending_ticket = conn.pending_ticket.take();
        let facts = LoginFacts {
            rewritten,
            online_mode,
        };
        let initial = match resolve_initial_mode(
            &ctx,
            &mut io,
            &routing,
            backend_targets.as_ref(),
            &mut pending_ticket,
            facts,
        )
        .await
        {
            Ok(InitialMode::Connected(initial)) => *initial,
            Ok(InitialMode::Denied(cause)) => {
                io.close();
                lifecycle.end(cause).await;
                return Ok(());
            }
            Err(e) => {
                io.close();
                lifecycle.end(DisconnectCause::Error).await;
                return Err(e);
            }
        };

        player.set_pending_server(initial.server.clone());
        if let ConnectionMode::Backend(ref backend) = initial.mode {
            player.set_connected_address(backend.server_address().cloned());
        }
        drop(pending_ticket);

        let session_id = ctx.profile().uuid;
        let mode_label = self.auth_strategy.mode_label();
        tracing::info!(
            session = %session_id,
            server = %initial.server,
            username = %ctx.username(),
            mode = mode_label,
            "session started"
        );

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_start(&self.metrics, initial.server.as_str(), mode_label);
        #[cfg(feature = "telemetry")]
        let session_server = initial.server.clone();

        conn.settle();
        let outcome = run_session_loop(&ctx, &mut io, initial).await;
        player.end_commands().await;

        let cause = match io.commands.take_kick(&mut io.client, registry, false) {
            Some(reason) => kicked(&mut io.client, reason, registry).await,
            None => disconnect_cause(&outcome, shutdown),
        };
        announce_shutdown(&ctx, &mut io.client, &cause).await;
        io.close();

        lifecycle.end(cause).await;

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            conn.connection_duration(),
            session_server.as_str(),
            mode_label,
        );

        log_proxy_loop_outcome(&session_id, &outcome);

        Ok(())
    }
}

struct Entry {
    routing: RoutingData,
    handshake: HandshakeData,
    online_mode: bool,
    resumed_server: Option<ServerId>,
}

async fn ended_before_start(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
    shutdown: &CancellationToken,
) -> Option<DisconnectCause> {
    let registry = ctx.registry();
    let refused = io
        .client_codec
        .close_reason()
        .or_else(|| io.server_codec.close_reason())
        .map(str::to_owned);
    if let Some(refused) = refused {
        tracing::warn!(reason = %refused, "a required codec filter refused the connection");
        let reason = Component::text(crate::filter::codec_chain::CODEC_FILTER_UNAVAILABLE);
        return Some(kicked(&mut io.client, reason, registry).await);
    }
    if let Some(reason) = io.commands.take_kick(&mut io.client, registry, false) {
        return Some(kicked(&mut io.client, reason, registry).await);
    }
    if ctx.token.is_cancelled() {
        let cause = cancelled_cause(shutdown, None);
        announce_shutdown(ctx, &mut io.client, &cause).await;
        return Some(cause);
    }
    None
}

async fn kicked(
    client: &mut ClientBridge,
    reason: Component,
    registry: &PacketRegistry,
) -> DisconnectCause {
    client.disconnect(&reason, registry).await.ok();
    DisconnectCause::Kicked {
        reason: Some(reason),
    }
}

async fn announce_shutdown(
    ctx: &SessionContext<'_>,
    client: &mut ClientBridge,
    cause: &DisconnectCause,
) {
    if !matches!(cause, DisconnectCause::Shutdown) {
        return;
    }
    let notice = ctx.services.relays.notice();
    if matches!(notice, ShutdownKind::Restart) && resume::send_back(ctx, client).await {
        return;
    }
    let reason = Component::text(notice.reason());
    client.disconnect(&reason, ctx.registry()).await.ok();
}

fn disconnect_cause(outcome: &ProxyLoopOutcome, shutdown: &CancellationToken) -> DisconnectCause {
    match outcome {
        ProxyLoopOutcome::ClientDisconnected => DisconnectCause::ClientQuit,
        ProxyLoopOutcome::Kicked { reason } => DisconnectCause::Kicked {
            reason: Some(reason.clone()),
        },
        ProxyLoopOutcome::BackendClosed { reason } => DisconnectCause::BackendClosed {
            reason: reason.clone(),
        },
        ProxyLoopOutcome::BackendKick(kick) => DisconnectCause::BackendClosed {
            reason: Some(kick.reason.clone()),
        },
        ProxyLoopOutcome::BackendDisconnected { .. } => {
            DisconnectCause::BackendClosed { reason: None }
        }
        ProxyLoopOutcome::Shutdown => cancelled_cause(shutdown, None),
        ProxyLoopOutcome::Error(_) | ProxyLoopOutcome::SwitchRequested { .. } => {
            DisconnectCause::Error
        }
    }
}

fn log_proxy_loop_outcome(session_id: &Uuid, outcome: &ProxyLoopOutcome) {
    match outcome {
        ProxyLoopOutcome::ClientDisconnected => {
            tracing::info!(session = %session_id, "client disconnected");
        }
        ProxyLoopOutcome::BackendDisconnected { reason } => {
            tracing::info!(session = %session_id, ?reason, "backend disconnected");
        }
        ProxyLoopOutcome::Shutdown => {
            tracing::debug!(session = %session_id, "shutdown");
        }
        ProxyLoopOutcome::Error(e) => {
            if e.is_expected_disconnect() {
                tracing::debug!(session = %session_id, error = %e, "session ended (expected)");
            } else {
                tracing::warn!(session = %session_id, error = %e, "session error");
            }
        }
        ProxyLoopOutcome::SwitchRequested { target, .. } => {
            tracing::info!(session = %session_id, %target, "server switch requested");
        }
        ProxyLoopOutcome::Kicked { reason } => {
            tracing::info!(session = %session_id, reason = %reason.to_plain(), "player kicked");
        }
        ProxyLoopOutcome::BackendKick(kick) => {
            tracing::info!(session = %session_id, reason = %kick, "backend kicked the player");
        }
        ProxyLoopOutcome::BackendClosed { reason } => {
            tracing::info!(
                session = %session_id,
                reason = reason.as_ref().map(Component::to_plain),
                "backend dropped the player"
            );
        }
    }
}
