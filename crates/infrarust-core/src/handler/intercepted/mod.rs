//! Unified handler for `ClientOnly` and `Offline` intercepted proxy modes.

mod auth;
mod initial_connect;
mod session_loop;

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::lifecycle::{
    DisconnectCause, GameProfileRequestEvent, LoginEvent, LoginResult,
};
use infrarust_api::player::Player;
use infrarust_api::services::ban_service::LoginAttempt;
use infrarust_api::types::{Component, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::commands::CommandInbox;
use crate::player::lifecycle::PlayerLifecycle;
use crate::player::{PlayerSession, SHUTDOWN_REASON};
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::proxy_loop::ProxyLoopOutcome;

use auth::{AuthResult, AuthStrategy};
use initial_connect::InitialMode;

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
        mut ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();
        let handshake = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = ctx.extensions.get::<LoginData>().cloned();
        let backend_targets = ctx
            .extensions
            .get::<crate::middleware::backend_selection::BackendTargets>()
            .cloned();

        let version = handshake.protocol_version;
        let api_version = infrarust_api::types::ProtocolVersion::new(version.0);
        let peer_addr = ctx.peer_addr;
        let remote_addr = ctx.client_addr();
        let connection_info = ctx.connection_info();
        let registry = &self.services.packet_registry;

        let mut client = ClientBridge::new(ctx.take_stream(), ctx.buffered_data.split(), version);

        let authenticated = self
            .auth_strategy
            .authenticate(
                &mut client,
                login_data.as_ref(),
                &self.services,
                version,
                remote_addr,
                &handshake.domain,
            )
            .await?;
        let online_mode = authenticated.online_mode;

        let request = self
            .services
            .event_bus
            .fire(GameProfileRequestEvent::new(
                authenticated.profile,
                online_mode,
                remote_addr,
                Some(handshake.domain.clone()),
                api_version,
            ))
            .await;
        let rewritten = request.is_modified();
        let profile = request.profile;

        let attempt = LoginAttempt::post_auth(
            ctx.client_ip,
            profile.username.clone(),
            profile.uuid,
            online_mode,
        )
        .virtual_host(handshake.domain.clone())
        .server(ServerId::new(routing.config_id.clone()));
        if let Some(reason) = self.services.ban_manager.refusal(&attempt).await {
            client.disconnect(&reason, registry).await.ok();
            return Ok(());
        }

        let session_token = shutdown.child_token();
        let (cmd_tx, cmd_rx) = PlayerSession::channel();
        let player = PlayerSession::new(
            PlayerId::new(ctx.connection_id),
            profile.clone(),
            api_version,
            remote_addr,
            None,
            true,
            online_mode,
            cmd_tx,
            session_token.clone(),
            crate::permissions::default_checker(),
            Arc::clone(&self.services.backend_load),
        )
        .with_permissions(Arc::clone(&self.services.permission_service))
        .with_virtual_host(handshake.domain.clone())
        .with_events(Arc::clone(&self.services.event_bus))
        .into_shared();

        player.setup_permissions(&self.services.event_bus).await;

        let login = self
            .services
            .event_bus
            .fire(LoginEvent::new(
                Arc::clone(&player) as Arc<dyn Player>,
                online_mode,
            ))
            .await;
        if let LoginResult::Denied { reason } = login.result() {
            tracing::info!(username = %profile.username, "login denied by a plugin");
            client.disconnect(reason, registry).await.ok();
            return Ok(());
        }

        let mut login_completed = false;
        if online_mode {
            auth::complete_login(&mut client, &profile, version, registry).await?;
            login_completed = true;
        }

        let lifecycle = PlayerLifecycle::begin(&self.services, Arc::clone(&player)).await;
        let mut commands =
            CommandInbox::new(cmd_rx).with_presentation(Arc::clone(player.presentation()));
        if let Some(reason) = commands.take_kick(&mut client, registry, false) {
            client.disconnect(&reason, registry).await.ok();
            lifecycle
                .end(DisconnectCause::Kicked {
                    reason: Some(reason),
                })
                .await;
            return Ok(());
        }
        if session_token.is_cancelled() {
            let cause = cancelled_cause(&shutdown);
            announce_shutdown(&mut client, &cause, registry).await;
            lifecycle.end(cause).await;
            return Ok(());
        }

        let auth_result = AuthResult::new(player.id(), profile, rewritten);
        let mut pending_ticket = ctx
            .extensions
            .remove::<crate::loadbalancer::PendingTicket>();

        let initial = match initial_connect::resolve_initial_mode(
            &mut client,
            &player,
            &auth_result,
            &mut login_completed,
            &routing,
            &handshake,
            backend_targets.as_ref(),
            &mut pending_ticket,
            version,
            &self.services,
            &self.backend_connector,
            &connection_info,
        )
        .await
        {
            Ok(initial) => initial,
            Err(e) => {
                lifecycle.end(DisconnectCause::Error).await;
                return Err(e);
            }
        };

        let (initial_mode, target_server_id, pending) = match initial {
            InitialMode::Connected {
                mode,
                server_id,
                pending,
            } => (*mode, server_id, pending),
            InitialMode::Denied(cause) => {
                lifecycle.end(cause).await;
                return Ok(());
            }
        };

        player.set_pending_server(target_server_id.clone());
        if let initial_connect::ConnectionMode::Backend(ref backend) = initial_mode {
            player.set_connected_address(backend.server_address().cloned());
        }
        // The session now owns the accounting for this address.
        drop(pending_ticket);

        let session_id = auth_result.player_uuid;
        let mode_label = self.auth_strategy.mode_label();
        tracing::info!(
            session = %session_id,
            server = %target_server_id,
            username = %auth_result.username,
            mode = mode_label,
            "session started"
        );

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_start(&self.metrics, target_server_id.as_str(), mode_label);

        let (mut client_codec_chain, mut server_codec_chain) =
            crate::filter::codec_chain::build_codec_chains(
                &self.services.codec_filter_registry,
                api_version,
                auth_result.player_id.as_u64(),
                peer_addr,
                Some(ctx.client_ip),
            );

        #[cfg(feature = "telemetry")]
        let session_server = target_server_id.clone();
        let outcome = session_loop::run_session_loop(
            &mut client,
            initial_mode,
            auth_result.player_id,
            &auth_result.api_profile,
            &auth_result.username,
            &handshake,
            version,
            peer_addr,
            Some(ctx.client_ip),
            target_server_id,
            pending,
            &player,
            &self.services,
            &self.backend_connector,
            session_token,
            &mut commands,
            &mut client_codec_chain,
            &mut server_codec_chain,
        )
        .await;
        player.end_commands().await;

        let cause = match commands.take_kick(&mut client, registry, false) {
            Some(reason) => {
                client.disconnect(&reason, registry).await.ok();
                DisconnectCause::Kicked {
                    reason: Some(reason),
                }
            }
            None => disconnect_cause(&outcome, &shutdown),
        };
        announce_shutdown(&mut client, &cause, registry).await;

        client_codec_chain.close();
        server_codec_chain.close();

        lifecycle.end(cause).await;

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            ctx.connection_duration(),
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
