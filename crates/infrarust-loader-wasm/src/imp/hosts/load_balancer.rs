use infrarust_api::permissions::Capability;
use infrarust_api::services::load_balancer::{BackendState, BackendStatus};
use infrarust_api::types::ServerId;

use crate::bindings::infrarust::plugin::events as we;
use crate::bindings::infrarust::plugin::load_balancer as wl;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::{HostResult, balancer_error};
use crate::store_state::PluginStoreState;

fn backend_status(status: &BackendStatus) -> wl::BackendStatus {
    wl::BackendStatus {
        address: convert::server_address_to_wit(&status.address),
        weight: status.weight,
        effective_weight: status.effective_weight,
        state: match status.state {
            BackendState::Healthy => we::BackendState::Healthy,
            BackendState::Probing => we::BackendState::Probing,
            BackendState::Draining => we::BackendState::Draining,
            _ => we::BackendState::Unhealthy,
        },
        active_connections: u64::try_from(status.active_connections).unwrap_or(u64::MAX),
        healthy_since_secs: status.healthy_since_secs,
        ejections: status.ejections,
        last_failure_secs_ago: status.last_failure_secs_ago,
    }
}

impl wl::Host for PluginStoreState {
    async fn strategy(&mut self, server: String) -> wasmtime::Result<HostResult<Option<String>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "load-balancer.strategy")?;
            Ok(self
                .services()?
                .load_balancer_service()
                .strategy(&ServerId::from(server)))
        })())
    }

    async fn backends(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Vec<wl::BackendStatus>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "load-balancer.backends")?;
            Ok(self
                .services()?
                .load_balancer_service()
                .backends(&ServerId::from(server))
                .iter()
                .map(backend_status)
                .collect())
        })())
    }

    async fn set_drained(
        &mut self,
        server: String,
        address: wt::ServerAddress,
        drained: bool,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(Capability::ServerManage, "load-balancer.set-drained")?;
            self.services()?
                .load_balancer_service()
                .set_drained(
                    &ServerId::from(server),
                    &convert::server_address_from_wit(address),
                    drained,
                )
                .map_err(|e| balancer_error(&e))
        })())
    }

    async fn reset_backend(
        &mut self,
        server: String,
        address: wt::ServerAddress,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(Capability::ServerManage, "load-balancer.reset-backend")?;
            self.services()?
                .load_balancer_service()
                .reset_backend(
                    &ServerId::from(server),
                    &convert::server_address_from_wit(address),
                )
                .map_err(|e| balancer_error(&e))
        })())
    }
}
