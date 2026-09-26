use infrarust_api::types::ServerId;

use crate::bindings::infrarust::plugin::config_service as wc;
use crate::convert;
use crate::host_error::{HostResult, config_write_error};
use crate::store_state::PluginStoreState;

impl wc::Host for PluginStoreState {
    async fn get_value(&mut self, key: String) -> wasmtime::Result<HostResult<Option<String>>> {
        Ok(self.config_value(&key))
    }

    async fn get_server(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Option<wc::ServerConfig>>> {
        Ok(self.server_config(server))
    }

    async fn list_servers(&mut self) -> wasmtime::Result<HostResult<Vec<wc::ServerConfig>>> {
        Ok(self.server_configs())
    }

    async fn get_server_document(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Option<String>>> {
        Ok(self.server_document(server))
    }

    async fn list_server_sources(&mut self) -> wasmtime::Result<HostResult<Vec<wc::ServerSource>>> {
        Ok(self.server_sources())
    }

    async fn get_proxy_config_document(&mut self) -> wasmtime::Result<HostResult<String>> {
        Ok(self.proxy_config_document())
    }

    async fn get_effective_proxy_config_document(
        &mut self,
    ) -> wasmtime::Result<HostResult<String>> {
        Ok(self.effective_proxy_config_document())
    }

    async fn write_proxy_config_document(
        &mut self,
        document: String,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok(self.write_proxy_document(&document))
    }
}

impl PluginStoreState {
    fn config_value(&mut self, key: &str) -> HostResult<Option<String>> {
        self.check("config-service", "get-value")?;
        Ok(self.services()?.config_service().get_value(key))
    }

    fn server_config(&mut self, server: String) -> HostResult<Option<wc::ServerConfig>> {
        self.check("config-service", "get-server")?;
        Ok(self
            .services()?
            .config_service()
            .get_server_config(&ServerId::from(server))
            .as_ref()
            .map(convert::server_config_to_wit))
    }

    fn server_configs(&mut self) -> HostResult<Vec<wc::ServerConfig>> {
        self.check("config-service", "list-servers")?;
        Ok(self
            .services()?
            .config_service()
            .get_all_server_configs()
            .iter()
            .map(convert::server_config_to_wit)
            .collect())
    }

    fn server_document(&mut self, server: String) -> HostResult<Option<String>> {
        self.check("config-service", "get-server-document")?;
        Ok(self
            .services()?
            .config_service()
            .get_server_document(&ServerId::from(server)))
    }

    fn server_sources(&mut self) -> HostResult<Vec<wc::ServerSource>> {
        self.check("config-service", "list-server-sources")?;
        Ok(self
            .services()?
            .config_service()
            .list_server_sources()
            .into_iter()
            .map(|source| wc::ServerSource {
                id: source.id,
                provider_id: source.provider_id,
                provider_type: source.provider_type,
                editable: source.editable,
            })
            .collect())
    }

    fn proxy_config_document(&mut self) -> HostResult<String> {
        self.check("config-service", "get-proxy-config-document")?;
        Ok(self
            .services()?
            .config_service()
            .get_proxy_config_document())
    }

    fn effective_proxy_config_document(&mut self) -> HostResult<String> {
        self.check("config-service", "get-effective-proxy-config-document")?;
        Ok(self
            .services()?
            .config_service()
            .get_effective_proxy_config_document())
    }

    fn write_proxy_document(&mut self, document: &str) -> HostResult<()> {
        self.check("config-service", "write-proxy-config-document")?;
        self.services()?
            .config_service()
            .write_proxy_config_document(document)
            .map_err(|e| config_write_error(&e))
    }
}
