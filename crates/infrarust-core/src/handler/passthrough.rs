use std::sync::Arc;

use infrarust_api::player::Player;
use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use super::forwarded::{Arrival, ForwardedLogin, Opening, Route, Wire};
use crate::error::CoreError;
use crate::middleware::backend_selection::BackendTargets;
use crate::pipeline::context::ConnectionContext;
use crate::pipeline::types::{HandshakeData, LoginData, RoutingData};
use crate::services::ProxyServices;

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
        let routing = ctx.require_extension::<RoutingData>("RoutingData")?.clone();
        let handshake = ctx
            .require_extension::<HandshakeData>("HandshakeData")?
            .clone();
        let login_data = ctx.extensions.get::<LoginData>().cloned();
        let addresses = BackendTargets::addresses_or_config(
            ctx.extensions.get::<BackendTargets>(),
            &routing.server_config,
        );

        let arrival = Arrival {
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
            .open(
                &mut ctx,
                arrival,
                Route { routing, addresses },
                Opening::Modern(&handshake),
            )
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

        let result = ready.forward(ctx.take_stream()).await;

        #[cfg(feature = "telemetry")]
        super::helpers::record_session_end(
            &self.metrics,
            ctx.connection_duration(),
            server.as_str(),
            "passthrough",
        );

        tracing::info!(
            session = %session_id,
            c2b = result.client_to_backend,
            b2c = result.backend_to_client,
            reason = ?result.reason,
            "session ended"
        );

        Ok(())
    }
}
