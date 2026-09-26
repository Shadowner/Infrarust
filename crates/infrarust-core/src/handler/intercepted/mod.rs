//! Unified handler for `ClientOnly` and `Offline` intercepted proxy modes.

pub(crate) mod auth;

use std::sync::Arc;

use infrarust_api::events::lifecycle::DisconnectCause;
use infrarust_api::player::{Player, session_task};
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::commands::CommandInbox;
use crate::player::{SHUTDOWN_REASON, SessionKind};
use crate::services::ProxyServices;
use crate::session::admission::{Admission, Admitted, Arrival, admit};
use crate::session::client_bridge::ClientBridge;
use crate::session::client_login::complete_login;
use crate::session::proxy_loop::ProxyLoopOutcome;

use crate::session::context::{SessionContext, SessionIo};
use crate::session::initial_connect::{
    ConnectionMode, InitialMode, LoginProgress, resolve_initial_mode,
};
use crate::session::session_loop::run_session_loop;
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
        let routing = conn
            .require_extension::<RoutingData>("RoutingData")?
            .clone();
        let handshake = conn
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = conn.extensions.get::<LoginData>().cloned();
        let backend_targets = conn
            .extensions
            .get::<crate::middleware::backend_selection::BackendTargets>()
            .cloned();

        let version = handshake.protocol_version;
        let api_version = infrarust_api::types::ProtocolVersion::new(version.0);
        let peer_addr = conn.peer_addr;
        let remote_addr = conn.client_addr();
        let connection_info = conn.connection_info();
        let registry = &self.services.packet_registry;

        let mut client = ClientBridge::new(conn.take_stream(), conn.buffered_data.split(), version);

        let (profile, online_mode) = match self
            .auth_strategy
            .authenticate(
                &mut client,
                login_data.as_ref(),
                &self.services,
                version,
                remote_addr,
                &handshake.domain,
            )
            .await?
        {
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
            protocol_version: api_version,
            domain: handshake.domain.clone(),
            origin: ServerId::new(routing.config_id.clone()),
        };
        let kind = SessionKind::Intercepted { online_mode };
        let Admitted {
            session: player,
            lifecycle,
            commands: cmd_rx,
            token: session_token,
            rewritten,
        } = match admit(&self.services, &conn, &shutdown, arrival, kind).await {
            Admission::Admitted(admitted) => admitted,
            Admission::Refused(reason) => {
                client.disconnect(&reason, registry).await.ok();
                return Ok(());
            }
        };
        let profile = player.game_profile().clone();

        let mut login_completed = false;
        if online_mode {
            if let Err(e) = complete_login(&mut client, &profile, version, registry).await {
                lifecycle.end(DisconnectCause::Error).await;
                return Err(e);
            }
            login_completed = true;
        }

        let commands =
            CommandInbox::new(cmd_rx).with_presentation(Arc::clone(player.presentation()));
        let (client_codec, server_codec) = crate::filter::codec_chain::build_codec_chains(
            &self.services.codec_filter_registry,
            api_version,
            player.id().as_u64(),
            peer_addr,
            Some(conn.client_ip),
        );
        let mut io = SessionIo {
            client,
            commands,
            client_codec,
            server_codec,
        };
        let ctx = SessionContext {
            services: &self.services,
            backend_connector: &self.backend_connector,
            session: Arc::clone(&player),
            handshake,
            connection_info,
            token: session_token,
        };

        if let Some(reason) = io.commands.take_kick(&mut io.client, registry, false) {
            io.client.disconnect(&reason, registry).await.ok();
            io.close();
            lifecycle
                .end(DisconnectCause::Kicked {
                    reason: Some(reason),
                })
                .await;
            return Ok(());
        }
        if ctx.token.is_cancelled() {
            let cause = cancelled_cause(&shutdown);
            announce_shutdown(&mut io.client, &cause, registry).await;
            io.close();
            lifecycle.end(cause).await;
            return Ok(());
        }

        let mut progress = LoginProgress {
            completed: login_completed,
            rewritten,
        };
        let mut pending_ticket = conn
            .extensions
            .remove::<crate::loadbalancer::PendingTicket>();

        let initial = match resolve_initial_mode(
            &ctx,
            &mut io,
            &routing,
            backend_targets.as_ref(),
            &mut pending_ticket,
            &mut progress,
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

        let outcome = run_session_loop(&ctx, &mut io, initial).await;
        player.end_commands().await;

        let cause = match io.commands.take_kick(&mut io.client, registry, false) {
            Some(reason) => {
                io.client.disconnect(&reason, registry).await.ok();
                DisconnectCause::Kicked {
                    reason: Some(reason),
                }
            }
            None => disconnect_cause(&outcome, &shutdown),
        };
        announce_shutdown(&mut io.client, &cause, registry).await;
        io.close();

        lifecycle.end(cause).await;

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            conn.connection_duration(),
            session_server.as_str(),
            mode_label,
        );

        super::helpers::log_proxy_loop_outcome(&session_id, &outcome);

        Ok(())
    }
}

async fn announce_shutdown(
    client: &mut ClientBridge,
    cause: &DisconnectCause,
    registry: &PacketRegistry,
) {
    if matches!(cause, DisconnectCause::Shutdown) {
        client
            .disconnect(&Component::text(SHUTDOWN_REASON), registry)
            .await
            .ok();
    }
}

fn cancelled_cause(shutdown: &CancellationToken) -> DisconnectCause {
    if shutdown.is_cancelled() {
        DisconnectCause::Shutdown
    } else {
        DisconnectCause::Kicked { reason: None }
    }
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
        ProxyLoopOutcome::Shutdown => cancelled_cause(shutdown),
        ProxyLoopOutcome::Error(_) | ProxyLoopOutcome::SwitchRequested { .. } => {
            DisconnectCause::Error
        }
    }
}
