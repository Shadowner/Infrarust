use std::sync::Arc;

use infrarust_api::player::Player;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use super::forwarded::{Applicant, ForwardedLogin, Opening, RelayEnd, Route, Wire};
use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::BackendTargets;
use crate::server::ProxyServices;

pub struct PassthroughHandler {
    backend_connector: Arc<BackendConnector>,
    services: ProxyServices,
    #[cfg(feature = "telemetry")]
    metrics: Option<Arc<crate::telemetry::ProxyMetrics>>,
}

impl PassthroughHandler {
    pub fn new(backend_connector: Arc<BackendConnector>, services: ProxyServices) -> Self {
        Self {
            backend_connector,
            services,
            #[cfg(feature = "telemetry")]
            metrics: None,
        }
    }

    #[cfg(feature = "telemetry")]
    pub fn with_metrics(mut self, metrics: Arc<crate::telemetry::ProxyMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    #[tracing::instrument(name = "proxy.session", skip_all, fields(mode = "passthrough"))]
    pub async fn handle(
        &self,
        mut ctx: ConnectionContext,
        shutdown: CancellationToken,
    ) -> Result<(), CoreError> {
        let routing = ctx.require_routing()?.clone();
        let handshake = ctx.require_handshake()?.clone();
        let login_data = ctx.login.clone();
        let origin = Route::forwarding(routing, |routing| {
            BackendTargets::addresses_or_config(
                ctx.backend_targets.as_ref(),
                &routing.server_config,
            )
        })
        .ok_or(CoreError::InvalidState(
            "a server where the proxy runs the login reached the passthrough handler",
        ))?;

        let applicant = Applicant {
            username: login_data
                .as_ref()
                .map(|d| d.username.clone())
                .unwrap_or_default(),
            claimed_uuid: login_data.as_ref().and_then(|d| d.player_uuid),
            protocol_version: infrarust_api::types::ProtocolVersion::new(
                handshake.protocol_version.0,
            ),
            domain: handshake.domain.clone(),
        };
        let login = ForwardedLogin {
            services: &self.services,
            connector: &self.backend_connector,
            shutdown: &shutdown,
            wire: Wire::Modern(handshake.protocol_version),
        };
        let Some(ready) = login
            .open(&mut ctx, applicant, origin, Opening::Modern(&handshake))
            .await?
        else {
            return Ok(());
        };

        let session_id = ready.player.profile().uuid;
        let server = ready.server.clone();
        tracing::info!(
            session = %session_id,
            server = %server,
            username = %ready.player.profile().username,
            mode = "passthrough",
            "session started"
        );

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_start(&self.metrics, server.as_str(), "passthrough");

        let result = ready.forward(&mut ctx).await;

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            ctx.connection_duration(),
            server.as_str(),
            "passthrough",
        );

        let (c2b, b2c) = (result.client_to_backend, result.backend_to_client);
        match &result.reason {
            RelayEnd::Ended(reason) => {
                tracing::info!(session = %session_id, c2b, b2c, ?reason, "session ended");
            }
            RelayEnd::Frozen => {
                tracing::debug!(session = %session_id, c2b, b2c, "session frozen for a hot restart");
            }
        }

        Ok(())
    }
}
