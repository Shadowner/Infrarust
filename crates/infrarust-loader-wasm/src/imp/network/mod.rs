pub(crate) mod http;
pub(crate) mod policy;
pub(crate) mod resolver;

use std::sync::Arc;
use std::time::Duration;

use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_config::WasmNetworkConfig;

pub(crate) use http::HttpHooks;
pub(crate) use policy::NetworkPolicy;
use resolver::SystemResolver;

pub(crate) const HOSTNAME_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) const DENIAL_LOG_BURST: u32 = 5;

pub(crate) fn policy_for(
    plugin_id: &str,
    config: Option<&WasmNetworkConfig>,
    capabilities: &CapabilitySet,
    host_call_timeout: Duration,
) -> Arc<NetworkPolicy> {
    let granted = capabilities.has(Capability::Network);
    let policy = NetworkPolicy::new(
        plugin_id.to_owned(),
        config,
        granted,
        host_call_timeout,
        Arc::new(SystemResolver),
    );
    match (config, granted) {
        (Some(_), false) => tracing::warn!(
            plugin = %plugin_id,
            "plugins.{plugin_id}.wasm.network is ignored: the plugin lacks the `network` capability, so every connection, lookup and HTTP request is refused"
        ),
        (_, true) if !policy.is_active() => tracing::warn!(
            plugin = %plugin_id,
            "plugin {plugin_id} has the `network` capability but an empty allow list (plugins.{plugin_id}.wasm.network.allow); every connection, lookup and HTTP request is refused"
        ),
        (Some(config), true) => tracing::info!(
            plugin = %plugin_id,
            rules = config.allow.len(),
            dns = policy.dns(),
            http = config.http,
            "wasm plugin network allow-list active"
        ),
        _ => {}
    }
    Arc::new(policy)
}

pub(crate) fn probe_policy(plugin_id: String) -> Arc<NetworkPolicy> {
    Arc::new(NetworkPolicy::disabled(plugin_id, Arc::new(SystemResolver)))
}
