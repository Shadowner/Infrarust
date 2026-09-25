use infrarust_api::permissions::Capability;
use infrarust_api::types::ServerId;

use crate::bindings::infrarust::plugin::config_service as wc;
use crate::convert;
use crate::host_error::HostResult;
use crate::store_state::PluginStoreState;

impl wc::Host for PluginStoreState {
    async fn get_value(&mut self, key: String) -> wasmtime::Result<HostResult<Option<String>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "config-service.get-value")?;
            Ok(self.services()?.config_service().get_value(&key))
        })())
    }

    async fn get_server(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Option<wc::ServerConfig>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "config-service.get-server")?;
            Ok(self
                .services()?
                .config_service()
                .get_server_config(&ServerId::from(server))
                .as_ref()
                .map(convert::server_config_to_wit))
        })())
    }

    async fn list_servers(&mut self) -> wasmtime::Result<HostResult<Vec<wc::ServerConfig>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "config-service.list-servers")?;
            Ok(self
                .services()?
                .config_service()
                .get_all_server_configs()
                .iter()
                .map(convert::server_config_to_wit)
                .collect())
        })())
    }
}
