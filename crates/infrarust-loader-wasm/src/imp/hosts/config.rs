use infrarust_api::permissions::Capability;
use infrarust_api::types::ServerId;

use crate::bindings::infrarust::plugin::config_service as wc;
use crate::convert;
use crate::host_error::{HostResult, config_write_error};
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

    async fn get_server_document(
        &mut self,
        server: String,
    ) -> wasmtime::Result<HostResult<Option<String>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "config-service.get-server-document")?;
            Ok(self
                .services()?
                .config_service()
                .get_server_document(&ServerId::from(server)))
        })())
    }

    async fn list_server_sources(&mut self) -> wasmtime::Result<HostResult<Vec<wc::ServerSource>>> {
        Ok((|| {
            self.check(Capability::ConfigRead, "config-service.list-server-sources")?;
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
        })())
    }

    async fn get_proxy_config_document(&mut self) -> wasmtime::Result<HostResult<String>> {
        Ok((|| {
            self.check(
                Capability::ConfigRead,
                "config-service.get-proxy-config-document",
            )?;
            Ok(self
                .services()?
                .config_service()
                .get_proxy_config_document())
        })())
    }

    async fn get_effective_proxy_config_document(
        &mut self,
    ) -> wasmtime::Result<HostResult<String>> {
        Ok((|| {
            self.check(
                Capability::ConfigRead,
                "config-service.get-effective-proxy-config-document",
            )?;
            Ok(self
                .services()?
                .config_service()
                .get_effective_proxy_config_document())
        })())
    }

    async fn write_proxy_config_document(
        &mut self,
        document: String,
    ) -> wasmtime::Result<HostResult<()>> {
        Ok((|| {
            self.check(
                Capability::ConfigWrite,
                "config-service.write-proxy-config-document",
            )?;
            self.services()?
                .config_service()
                .write_proxy_config_document(&document)
                .map_err(|e| config_write_error(&e))
        })())
    }
}
