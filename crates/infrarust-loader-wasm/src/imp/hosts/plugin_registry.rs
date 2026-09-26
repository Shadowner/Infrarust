use infrarust_api::services::plugin_registry::PluginInfo;

use crate::bindings::infrarust::plugin::plugin_registry as wr;
use crate::bindings::infrarust::plugin::types as wt;
use crate::store_state::PluginStoreState;

fn plugin_info(info: PluginInfo) -> wr::PluginInfo {
    let state = info.state.as_str().to_string();
    let meta = info.metadata;
    wr::PluginInfo {
        id: meta.id,
        name: meta.name,
        version: meta.version,
        authors: meta.authors,
        description: meta.description,
        state,
        dependencies: meta
            .dependencies
            .into_iter()
            .map(|dependency| wt::PluginDependency {
                id: dependency.id,
                optional: dependency.optional,
            })
            .collect(),
    }
}

impl wr::Host for PluginStoreState {
    async fn list(&mut self) -> wasmtime::Result<Vec<wr::PluginInfo>> {
        Ok(self
            .ctx()
            .map(|ctx| {
                ctx.plugin_registry()
                    .list_plugin_info()
                    .into_iter()
                    .map(plugin_info)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get(&mut self, id: String) -> wasmtime::Result<Option<wr::PluginInfo>> {
        Ok(self
            .ctx()
            .and_then(|ctx| ctx.plugin_registry().plugin_info(&id))
            .map(plugin_info))
    }
}
