use infrarust_api::permissions::Capability;
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
        Ok((|| {
            self.check(Capability::ServerManage, "server-manager.get-state")?;
            Ok(self
                .services()?
                .server_manager()
                .get_state(&ServerId::from(server))
                .map(convert::server_state_to_wit))
        })())
    }

    async fn start(&mut self, server: String) -> wasmtime::Result<HostResult<()>> {
        if let Err(error) = self.check(Capability::ServerManage, "server-manager.start") {
            return Ok(Err(error));
        }
        let ctx = match self.services() {
            Ok(ctx) => ctx,
            Err(error) => return Ok(Err(error)),
        };
        let server = ServerId::from(server);
        Ok(await_service(
            self.service_call_limit(),
            ctx.server_manager().start(&server),
        )
        .await)
    }

    async fn stop(&mut self, server: String) -> wasmtime::Result<HostResult<()>> {
        if let Err(error) = self.check(Capability::ServerManage, "server-manager.stop") {
            return Ok(Err(error));
        }
        let ctx = match self.services() {
            Ok(ctx) => ctx,
            Err(error) => return Ok(Err(error)),
        };
        let server = ServerId::from(server);
        Ok(await_service(
            self.service_call_limit(),
            ctx.server_manager().stop(&server),
        )
        .await)
    }

    async fn list(&mut self) -> wasmtime::Result<HostResult<Vec<ws::ServerStatus>>> {
        Ok((|| {
            self.check(Capability::ServerManage, "server-manager.list")?;
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
        })())
    }
}
