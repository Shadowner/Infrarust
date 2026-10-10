use infrarust_api::types::Component;

use super::parse_text;
use crate::bindings::infrarust::plugin::text as wtext;
use crate::bindings::infrarust::plugin::types as wt;
use crate::component;
use crate::host_error::{HostResult, host_error};
use crate::store_state::PluginStoreState;

impl wtext::Host for PluginStoreState {
    async fn parse_json(&mut self, json: String) -> wasmtime::Result<HostResult<wt::Component>> {
        Ok(Component::from_json(&json)
            .map(|parsed| component::to_wit(&parsed))
            .map_err(|e| host_error(wt::ErrorKind::InvalidArgument, e.to_string())))
    }

    async fn parse_legacy(&mut self, legacy: String) -> wasmtime::Result<wt::Component> {
        Ok(component::to_wit(&Component::from_legacy(&legacy)))
    }

    async fn to_json(&mut self, value: wt::Component) -> wasmtime::Result<HostResult<String>> {
        Ok(parse_text(&value).map(|parsed| parsed.to_json()))
    }

    async fn to_plain(&mut self, value: wt::Component) -> wasmtime::Result<HostResult<String>> {
        Ok(parse_text(&value).map(|parsed| parsed.to_plain()))
    }
}
