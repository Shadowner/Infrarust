use std::collections::{HashMap, HashSet};
use std::time::Duration;

use infrarust_config::{
    EventsConfig, ProxyConfig, WasmConfig, WasmLimits, WasmMount, WasmNetworkConfig,
    WasmRecoveryConfig,
};

#[derive(Debug, Clone)]
pub struct WasmLoaderConfig {
    epoch_tick: Duration,
    event_handler_timeout: Duration,
    defaults: WasmLimits,
    plugins: HashMap<String, WasmLimits>,
    strict_capabilities: HashSet<String>,
    network: HashMap<String, WasmNetworkConfig>,
    mounts: HashMap<String, Vec<WasmMount>>,
}

impl WasmLoaderConfig {
    #[must_use]
    pub fn from_proxy_config(config: &ProxyConfig) -> Self {
        let plugins = config
            .plugins
            .iter()
            .filter_map(|(id, plugin)| {
                plugin
                    .wasm
                    .as_ref()
                    .map(|overrides| (id.clone(), config.wasm.limits_for(Some(overrides))))
            })
            .collect();
        let strict_capabilities = config
            .plugins
            .iter()
            .filter(|(_, plugin)| plugin.strict_capabilities)
            .map(|(id, _)| id.clone())
            .collect();
        let wasm_tables = || {
            config
                .plugins
                .iter()
                .filter_map(|(id, plugin)| plugin.wasm.as_ref().map(|wasm| (id, wasm)))
        };
        let network = wasm_tables()
            .filter_map(|(id, wasm)| wasm.network.clone().map(|network| (id.clone(), network)))
            .collect();
        let mounts = wasm_tables()
            .filter(|(_, wasm)| !wasm.mounts.is_empty())
            .map(|(id, wasm)| (id.clone(), wasm.mounts.clone()))
            .collect();
        Self {
            epoch_tick: config.wasm.epoch_tick,
            event_handler_timeout: config.events.handler_timeout,
            defaults: config.wasm.limits(),
            plugins,
            strict_capabilities,
            network,
            mounts,
        }
    }

    #[must_use]
    pub fn epoch_tick(&self) -> Duration {
        self.epoch_tick
    }

    #[must_use]
    pub fn limits_for(&self, plugin_id: &str) -> WasmLimits {
        self.plugins
            .get(plugin_id)
            .copied()
            .unwrap_or(self.defaults)
    }

    #[must_use]
    pub fn strict_capabilities(&self, plugin_id: &str) -> bool {
        self.strict_capabilities.contains(plugin_id)
    }

    pub(crate) fn network_for(&self, plugin_id: &str) -> Option<&WasmNetworkConfig> {
        self.network.get(plugin_id)
    }

    pub(crate) fn mounts_for(&self, plugin_id: &str) -> &[WasmMount] {
        self.mounts.get(plugin_id).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn sandbox_for(&self, plugin_id: &str) -> SandboxLimits {
        SandboxLimits::new(
            &self.limits_for(plugin_id),
            self.epoch_tick,
            self.event_handler_timeout,
        )
    }

    pub(crate) fn default_sandbox(&self) -> SandboxLimits {
        SandboxLimits::new(&self.defaults, self.epoch_tick, self.event_handler_timeout)
    }
}

impl Default for WasmLoaderConfig {
    fn default() -> Self {
        let wasm = WasmConfig::default();
        Self {
            epoch_tick: wasm.epoch_tick,
            event_handler_timeout: EventsConfig::default().handler_timeout,
            defaults: wasm.limits(),
            plugins: HashMap::new(),
            strict_capabilities: HashSet::new(),
            network: HashMap::new(),
            mounts: HashMap::new(),
        }
    }
}

impl From<&ProxyConfig> for WasmLoaderConfig {
    fn from(config: &ProxyConfig) -> Self {
        Self::from_proxy_config(config)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SandboxLimits {
    pub(crate) memory_bytes: usize,
    pub(crate) max_epoch_yields: u32,
    pub(crate) codec_deadline_ticks: u64,
    pub(crate) host_call_timeout: Duration,
    pub(crate) max_call_duration: Duration,
    pub(crate) queue_capacity: usize,
    pub(crate) event_budget: Duration,
    pub(crate) recovery: WasmRecoveryConfig,
}

impl SandboxLimits {
    fn new(limits: &WasmLimits, epoch_tick: Duration, event_budget: Duration) -> Self {
        Self {
            memory_bytes: limits.memory_limit_bytes(),
            max_epoch_yields: u32::try_from(ticks(limits.cpu_budget, epoch_tick))
                .unwrap_or(u32::MAX),
            codec_deadline_ticks: ticks(limits.codec_cpu_budget, epoch_tick),
            host_call_timeout: limits.host_call_timeout,
            max_call_duration: limits.max_call_duration,
            queue_capacity: limits
                .queue_capacity
                .clamp(1, tokio::sync::Semaphore::MAX_PERMITS),
            event_budget,
            recovery: limits.recovery,
        }
    }
}

impl Default for SandboxLimits {
    fn default() -> Self {
        WasmLoaderConfig::default().default_sandbox()
    }
}

fn ticks(budget: Duration, epoch_tick: Duration) -> u64 {
    let tick = epoch_tick.as_nanos().max(1);
    u64::try_from(budget.as_nanos().div_ceil(tick))
        .unwrap_or(u64::MAX)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budgets_match_the_previous_hardcoded_sandbox() {
        let sandbox = SandboxLimits::default();
        assert_eq!(sandbox.memory_bytes, 64 * 1024 * 1024);
        assert_eq!(sandbox.max_epoch_yields, 60);
        assert_eq!(sandbox.codec_deadline_ticks, 16);
        assert_eq!(sandbox.host_call_timeout, Duration::from_secs(30));
        assert_eq!(sandbox.max_call_duration, Duration::from_secs(60));
        assert_eq!(sandbox.queue_capacity, 1024);
        assert_eq!(sandbox.event_budget, Duration::from_secs(10));
        assert_eq!(sandbox.recovery, WasmRecoveryConfig::default());
    }

    #[test]
    fn recovery_limits_follow_the_wasm_section_and_plugin_overrides() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm.recovery]
            max_restarts = 3
            backoff_initial = "2s"

            [plugins.flaky.wasm.recovery]
            max_restarts = 1
            "#,
        )
        .unwrap();
        let loader = WasmLoaderConfig::from_proxy_config(&config);
        let defaults = loader.default_sandbox().recovery;
        assert_eq!(defaults.max_restarts, 3);
        assert_eq!(defaults.backoff_initial, Duration::from_secs(2));
        assert_eq!(defaults.window, Duration::from_secs(300));
        let flaky = loader.sandbox_for("flaky").recovery;
        assert_eq!(flaky.max_restarts, 1);
        assert_eq!(flaky.backoff_initial, Duration::from_secs(2));
        assert_eq!(loader.sandbox_for("other").recovery, defaults);
    }

    #[test]
    fn events_run_under_the_event_bus_handler_timeout() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [events]
            handler_timeout = "300ms"

            [plugins.small.wasm]
            max_call_duration = "2s"
            "#,
        )
        .unwrap();
        let loader = WasmLoaderConfig::from_proxy_config(&config);
        assert_eq!(
            loader.default_sandbox().event_budget,
            Duration::from_millis(300)
        );
        assert_eq!(
            loader.sandbox_for("small").event_budget,
            Duration::from_millis(300)
        );
        assert_eq!(
            loader.sandbox_for("small").max_call_duration,
            Duration::from_secs(2)
        );
    }

    #[test]
    fn budgets_are_counted_in_epoch_ticks_rounding_up() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm]
            epoch_tick = "20ms"
            cpu_budget = "1s"
            codec_cpu_budget = "30ms"
            "#,
        )
        .unwrap();
        let sandbox = WasmLoaderConfig::from_proxy_config(&config).default_sandbox();
        assert_eq!(sandbox.max_epoch_yields, 50);
        assert_eq!(sandbox.codec_deadline_ticks, 2);
    }

    #[test]
    fn strict_capabilities_is_read_per_plugin() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [plugins.locked]
            strict_capabilities = true

            [plugins.relaxed]
            strict_capabilities = false
            permissions = ["ban"]
            "#,
        )
        .unwrap();
        let loader = WasmLoaderConfig::from_proxy_config(&config);
        assert!(loader.strict_capabilities("locked"));
        assert!(!loader.strict_capabilities("relaxed"));
        assert!(!loader.strict_capabilities("unknown"));
        assert!(!WasmLoaderConfig::default().strict_capabilities("locked"));
    }

    #[test]
    fn network_and_mounts_are_read_per_plugin() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [plugins.net.wasm.network]
            allow = ["127.0.0.1:5432"]
            http = false

            [[plugins.files.wasm.mounts]]
            host = "/srv/files"
            guest = "/files"

            [plugins.plain.wasm]
            memory_limit_mb = 8
            "#,
        )
        .unwrap();
        let loader = WasmLoaderConfig::from_proxy_config(&config);
        let net = loader.network_for("net").unwrap();
        assert_eq!(net.allow[0].to_string(), "127.0.0.1:5432");
        assert!(!net.http);
        assert!(loader.network_for("files").is_none());
        assert!(loader.network_for("plain").is_none());
        assert_eq!(loader.mounts_for("files")[0].guest, "/files");
        assert!(loader.mounts_for("net").is_empty());
        assert!(loader.mounts_for("unknown").is_empty());
        assert!(WasmLoaderConfig::default().network_for("net").is_none());
    }

    #[test]
    fn a_plugin_override_only_changes_that_plugin() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm]
            queue_capacity = 64

            [plugins.small.wasm]
            memory_limit_mb = 8

            [plugins.plain]
            permissions = ["ban"]
            "#,
        )
        .unwrap();
        let loader = WasmLoaderConfig::from_proxy_config(&config);
        let small = loader.sandbox_for("small");
        assert_eq!(small.memory_bytes, 8 * 1024 * 1024);
        assert_eq!(small.queue_capacity, 64);
        assert_eq!(loader.sandbox_for("plain"), loader.default_sandbox());
        assert_eq!(loader.sandbox_for("unknown"), loader.default_sandbox());
        assert_eq!(loader.default_sandbox().memory_bytes, 64 * 1024 * 1024);
    }
}
