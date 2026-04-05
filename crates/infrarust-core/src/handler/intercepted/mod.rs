//! Unified handler for `ClientOnly` and `Offline` intercepted proxy modes.

mod auth;
mod initial_connect;
mod session_loop;

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::lifecycle::{PermissionsSetupEvent, PermissionsSetupResult};
use infrarust_api::permissions::PermissionChecker;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::player::PlayerSession;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

use auth::AuthStrategy;
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

        let version = handshake.protocol_version;
        let peer_addr = ctx.peer_addr;
        let connection_info = ctx.connection_info();

        let mut client = ClientBridge::new(ctx.take_stream(), ctx.buffered_data.split(), version);

        let auth_result = self
            .auth_strategy
            .authenticate(
                &mut client,
                login_data.as_ref(),
                &self.services,
                version,
                peer_addr,
                &handshake.domain,
            )
            .await?;

        let mut login_completed = auth_result.login_completed;

        let initial = initial_connect::resolve_initial_mode(
            &mut client,
            &auth_result,
            &mut login_completed,
            &routing,
            &handshake,
            version,
            &self.services,
            &self.backend_connector,
            &connection_info,
        )
        .await?;

        let (initial_mode, target_server_id) = match initial {
            InitialMode::Connected { mode, server_id } => (*mode, server_id),
            InitialMode::Denied => return Ok(()),
        };

        let default_checker: Arc<dyn PermissionChecker> = if auth_result.online_mode {
            Arc::new(
                self.services
                    .permission_service
                    .build_checker(auth_result.player_uuid),
            )
        } else {
            crate::permissions::default_checker()
        };

        let perm_event = PermissionsSetupEvent::new(
            auth_result.player_id,
            auth_result.api_profile.clone(),
            auth_result.online_mode,
        );
        let perm_event = self.services.event_bus.fire(perm_event).await;
        let permission_checker =
            if let PermissionsSetupResult::Custom(checker) = perm_event.result() {
                checker.clone()
            } else {
                default_checker
            };

        let session_token = shutdown.child_token();
        let (cmd_tx, cmd_rx) = PlayerSession::channel();

        let player_session = Arc::new(PlayerSession::new(
            auth_result.player_id,
            auth_result.api_profile.clone(),
            infrarust_api::types::ProtocolVersion::new(version.0),
            ctx.peer_addr,
            Some(infrarust_api::types::ServerId::new(
                routing.config_id.clone(),
            )),
            true, // active: intercepted modes support packet injection
            auth_result.online_mode,
            cmd_tx,
            session_token.clone(),
            permission_checker,
        ));

        let session_id = self.services.connection_registry.register(player_session);

        let mode_label = self.auth_strategy.mode_label();
        tracing::info!(
            session = %session_id,
            server = %routing.config_id,
            username = %auth_result.username,
            mode = mode_label,
            "session started"
        );

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_start(&self.metrics, &routing.config_id, mode_label);

        let (mut client_codec_chain, mut server_codec_chain) =
            crate::filter::codec_chain::build_codec_chains(
                &self.services.codec_filter_registry,
                infrarust_api::types::ProtocolVersion::new(version.0),
                auth_result.player_id.as_u64(),
                ctx.peer_addr,
                Some(ctx.client_ip),
            );

        let mut cmd_rx = cmd_rx;
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
            &session_id,
            &self.services,
            &self.backend_connector,
            session_token,
            &mut cmd_rx,
            &mut client_codec_chain,
            &mut server_codec_chain,
        )
        .await;

        super::helpers::fire_disconnect_event(
            &self.services.event_bus,
            auth_result.player_id,
            auth_result.username.clone(),
            Some(infrarust_api::types::ServerId::new(
                routing.config_id.clone(),
            )),
        )
        .await;

        let _ = self.services.connection_registry.unregister(&session_id);

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            ctx.connection_duration(),
            &routing.config_id,
            mode_label,
        );

        super::helpers::log_proxy_loop_outcome(&session_id, &outcome);

        Ok(())
    }
}
