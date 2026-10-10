use std::time::Duration;

use infrarust_api::plugin::PluginHealth;
use infrarust_api::services::plugin_registry::PluginInfo;

use crate::bindings::infrarust::plugin::plugin_registry as wr;
use crate::bindings::infrarust::plugin::types as wt;
use crate::store_state::PluginStoreState;

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn health_to_wit(health: PluginHealth) -> Option<wr::PluginHealth> {
    Some(match health {
        PluginHealth::Healthy => wr::PluginHealth::Healthy,
        PluginHealth::Recovering { retry_in } => wr::PluginHealth::Recovering(retry_in.map(millis)),
        PluginHealth::Quarantined { retry_in } => wr::PluginHealth::Quarantined(millis(retry_in)),
        PluginHealth::Stopped => wr::PluginHealth::Stopped,
        _ => return None,
    })
}

fn plugin_info(info: PluginInfo) -> wr::PluginInfo {
    let state = info.state.as_str().to_string();
    let health = info
        .runtime
        .as_ref()
        .and_then(|runtime| health_to_wit(runtime.health));
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
        health,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use infrarust_api::plugin::{
        PluginMetadata, PluginQueueStats, PluginRuntimeStatus, PluginState,
    };

    use super::*;

    fn info(runtime: Option<PluginRuntimeStatus>) -> PluginInfo {
        PluginInfo {
            metadata: PluginMetadata::new("guest", "Guest", "1.0.0"),
            state: PluginState::Enabled,
            runtime,
        }
    }

    fn status(health: PluginHealth) -> PluginRuntimeStatus {
        PluginRuntimeStatus::new(health, 3, PluginQueueStats::default())
    }

    #[test]
    fn a_wasm_plugin_shows_its_health_and_a_native_one_has_none() {
        assert_eq!(plugin_info(info(None)).health, None);
        assert_eq!(
            plugin_info(info(Some(status(PluginHealth::Healthy)))).health,
            Some(wr::PluginHealth::Healthy)
        );
        assert_eq!(
            plugin_info(info(Some(status(PluginHealth::Quarantined {
                retry_in: Duration::from_secs(5),
            }))))
            .health,
            Some(wr::PluginHealth::Quarantined(5_000))
        );
        assert_eq!(
            plugin_info(info(Some(status(PluginHealth::Recovering {
                retry_in: None,
            }))))
            .health,
            Some(wr::PluginHealth::Recovering(None))
        );
        let stopped = plugin_info(info(Some(status(PluginHealth::Stopped))));
        assert_eq!(stopped.health, Some(wr::PluginHealth::Stopped));
        assert_eq!(stopped.state, "enabled");
    }
}
