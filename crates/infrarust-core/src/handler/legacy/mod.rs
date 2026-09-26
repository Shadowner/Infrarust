mod login;
mod ping;

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::error::CoreError;
use crate::pipeline::context::ConnectionContext;
use crate::services::ProxyServices;

pub struct LegacyHandler {
    services: ProxyServices,
    backend_connector: Arc<BackendConnector>,
    shutdown: CancellationToken,
}

impl LegacyHandler {
    pub fn new(
        services: ProxyServices,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            services,
            backend_connector,
            shutdown,
        }
    }

    pub async fn handle(&self, ctx: &mut ConnectionContext) -> Result<(), CoreError> {
        let first_byte = ctx.buffered_data.first().copied().unwrap_or(0);

        match first_byte {
            0xFE => self.handle_ping(ctx).await,
            0x02 => self.handle_login(ctx).await,
            _ => {
                tracing::debug!(byte = first_byte, "unknown legacy first byte");
                Ok(())
            }
        }
    }
}
