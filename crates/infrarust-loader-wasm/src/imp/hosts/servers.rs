use infrarust_api::types::ServerId;

use super::await_service;
use crate::bindings::infrarust::plugin::server_manager as ws;
use crate::bindings::infrarust::plugin::types as wt;
use crate::convert;
use crate::host_error::HostResult;
use crate::store_state::PluginStoreState;

impl ws::Host for PluginStoreState {
    async fn get_state(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Option<wt::ServerState>>> {
        Ok(self.server_state(server))
    }

    async fn start(&mut self, server: String) -> wasmtime::Result<HostResult<()>> {
        Ok(self.start_server(server).await)
    }

    async fn stop(&mut self, server: String) -> wasmtime::Result<HostResult<()>> {
        Ok(self.stop_server(server).await)
    }

    async fn list(&mut self) -> wasmtime::Result<HostResult<Vec<ws::ServerStatus>>> {
        Ok(self.server_statuses())
    }
}

impl PluginStoreState {
    fn server_state(&mut self, server: String) -> HostResult<Option<wt::ServerState>> {
        self.check("server-manager", "get-state")?;
        Ok(self
            .services()?
            .server_manager()
            .get_state(&ServerId::from(server))
            .map(convert::server_state_to_wit))
    }

    async fn start_server(&mut self, server: String) -> HostResult<()> {
        self.check("server-manager", "start")?;
        let ctx = self.services()?;
        let server = ServerId::from(server);
        await_service(
            self.service_call_limit(),
            ctx.server_manager().start(&server),
        )
        .await
    }

    async fn stop_server(&mut self, server: String) -> HostResult<()> {
        self.check("server-manager", "stop")?;
        let ctx = self.services()?;
        let server = ServerId::from(server);
        await_service(
            self.service_call_limit(),
            ctx.server_manager().stop(&server),
        )
        .await
    }

    fn server_statuses(&mut self) -> HostResult<Vec<ws::ServerStatus>> {
        self.check("server-manager", "list")?;
        Ok(self
            .services()?
            .server_manager()
            .get_all_servers()
            .into_iter()
            .map(|(server, state)| ws::ServerStatus {
                server: server.as_str().to_owned(),
                state: convert::server_state_to_wit(state),
            })
            .collect())
    }
}
