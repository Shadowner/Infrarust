mod login;
mod ping;

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use infrarust_transport::BackendConnector;

use crate::server::ProxyServices;
use crate::status::StatusCache;
use crate::status::relay_gate::RelayGate;

pub struct LegacyHandler {
    services: ProxyServices,
    backend_connector: Arc<BackendConnector>,
    shutdown: CancellationToken,
    status_cache: Arc<StatusCache>,
    relay_gate: Arc<RelayGate>,
}

impl LegacyHandler {
    pub fn new(
        services: ProxyServices,
        backend_connector: Arc<BackendConnector>,
        shutdown: CancellationToken,
        status_cache: Arc<StatusCache>,
        relay_gate: Arc<RelayGate>,
    ) -> Self {
        Self {
            services,
            backend_connector,
            shutdown,
            status_cache,
            relay_gate,
        }
    }
}
