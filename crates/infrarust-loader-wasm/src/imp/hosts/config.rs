use infrarust_api::types::ServerId;
use infrarust_config::secrets::{self, PluginScopeError};

use crate::bindings::infrarust::plugin::config_service as wc;
use crate::bindings::infrarust::plugin::types::{ErrorKind, HostError};
use crate::convert;
use crate::host_error::{HostResult, config_write_error, host_error};
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
        self.check(gate!("config-service", "get-value"))?;
        if secrets::names_another_plugin(key, self.plugin_id()) {
            return Err(host_error(
                ErrorKind::PermissionDenied,
                format!("`{key}` belongs to another plugin's configuration"),
            ));
        }
        let value = self.services()?.config_service().get_value(key);
        if key != secrets::PLUGINS {
            return Ok(value);
        }
        value
            .map(|table| secrets::plugins_value_view(&table, self.plugin_id()))
            .transpose()
            .map_err(|e| scope_error(&e))
    }

    fn server_config(&mut self, server: String) -> HostResult<Option<wc::ServerConfig>> {
        self.check(gate!("config-service", "get-server"))?;
        Ok(self
            .services()?
            .config_service()
            .get_server_config(&ServerId::from(server))
            .as_ref()
            .map(convert::server_config_to_wit))
    }

    fn server_configs(&mut self) -> HostResult<Vec<wc::ServerConfig>> {
        self.check(gate!("config-service", "list-servers"))?;
        Ok(self
            .services()?
            .config_service()
            .get_all_server_configs()
            .iter()
            .map(convert::server_config_to_wit)
            .collect())
    }

    fn server_document(&mut self, server: String) -> HostResult<Option<String>> {
        self.check(gate!("config-service", "get-server-document"))?;
        Ok(self
            .services()?
            .config_service()
            .get_server_document(&ServerId::from(server)))
    }

    fn server_sources(&mut self) -> HostResult<Vec<wc::ServerSource>> {
        self.check(gate!("config-service", "list-server-sources"))?;
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
        self.check(gate!("config-service", "get-proxy-config-document"))?;
        let document = self
            .services()?
            .config_service()
            .get_proxy_config_document();
        secrets::plugin_view(&document, self.plugin_id()).map_err(|e| scope_error(&e))
    }

    fn effective_proxy_config_document(&mut self) -> HostResult<String> {
        self.check(gate!(
            "config-service",
            "get-effective-proxy-config-document"
        ))?;
        let document = self
            .services()?
            .config_service()
            .get_effective_proxy_config_document();
        secrets::plugin_view(&document, self.plugin_id()).map_err(|e| scope_error(&e))
    }

    fn write_proxy_document(&mut self, document: &str) -> HostResult<()> {
        self.check(gate!("config-service", "write-proxy-config-document"))?;
        let service = self.services()?.config_service();
        let current = service.get_proxy_config_document();
        let document = secrets::plugin_write(document, &current, self.plugin_id())
            .map_err(|e| scope_error(&e))?;
        service
            .write_proxy_config_document(&document)
            .map_err(|e| config_write_error(&e))
    }
}

fn scope_error(error: &PluginScopeError) -> HostError {
    let kind = match error {
        PluginScopeError::Invalid(_) => ErrorKind::InvalidArgument,
        PluginScopeError::OtherPlugins(_) | PluginScopeError::OwnGrant(_) => {
            ErrorKind::PermissionDenied
        }
        PluginScopeError::Unreadable(_) => ErrorKind::Internal,
    };
    host_error(kind, error.to_string())
}
