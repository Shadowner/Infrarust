use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::defaults;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmConfig {
    #[serde(default = "defaults::wasm_epoch_tick")]
    #[serde(with = "humantime_serde")]
    pub epoch_tick: Duration,

    #[serde(default = "defaults::wasm_memory_limit_mb")]
    pub memory_limit_mb: u32,

    #[serde(default = "defaults::wasm_cpu_budget")]
    #[serde(with = "humantime_serde")]
    pub cpu_budget: Duration,

    #[serde(default = "defaults::wasm_codec_cpu_budget")]
    #[serde(with = "humantime_serde")]
    pub codec_cpu_budget: Duration,

    #[serde(default = "defaults::wasm_host_call_timeout")]
    #[serde(with = "humantime_serde")]
    pub host_call_timeout: Duration,

    #[serde(default = "defaults::wasm_max_call_duration")]
    #[serde(with = "humantime_serde")]
    pub max_call_duration: Duration,

    #[serde(default = "defaults::wasm_queue_capacity")]
    pub queue_capacity: usize,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            epoch_tick: defaults::wasm_epoch_tick(),
            memory_limit_mb: defaults::wasm_memory_limit_mb(),
            cpu_budget: defaults::wasm_cpu_budget(),
            codec_cpu_budget: defaults::wasm_codec_cpu_budget(),
            host_call_timeout: defaults::wasm_host_call_timeout(),
            max_call_duration: defaults::wasm_max_call_duration(),
            queue_capacity: defaults::wasm_queue_capacity(),
        }
    }
}

impl WasmConfig {
    #[must_use]
    pub const fn limits(&self) -> WasmLimits {
        WasmLimits {
            memory_limit_mb: self.memory_limit_mb,
            cpu_budget: self.cpu_budget,
            codec_cpu_budget: self.codec_cpu_budget,
            host_call_timeout: self.host_call_timeout,
            max_call_duration: self.max_call_duration,
            queue_capacity: self.queue_capacity,
        }
    }

    #[must_use]
    pub fn limits_for(&self, overrides: Option<&PluginWasmConfig>) -> WasmLimits {
        let base = self.limits();
        overrides.map_or(base, |o| o.apply(base))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginWasmConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub cpu_budget: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub codec_cpu_budget: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub host_call_timeout: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub max_call_duration: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_capacity: Option<usize>,
}

impl PluginWasmConfig {
    #[must_use]
    pub fn apply(&self, base: WasmLimits) -> WasmLimits {
        WasmLimits {
            memory_limit_mb: self.memory_limit_mb.unwrap_or(base.memory_limit_mb),
            cpu_budget: self.cpu_budget.unwrap_or(base.cpu_budget),
            codec_cpu_budget: self.codec_cpu_budget.unwrap_or(base.codec_cpu_budget),
            host_call_timeout: self.host_call_timeout.unwrap_or(base.host_call_timeout),
            max_call_duration: self.max_call_duration.unwrap_or(base.max_call_duration),
            queue_capacity: self.queue_capacity.unwrap_or(base.queue_capacity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmLimits {
    pub memory_limit_mb: u32,
    pub cpu_budget: Duration,
    pub codec_cpu_budget: Duration,
    pub host_call_timeout: Duration,
    pub max_call_duration: Duration,
    pub queue_capacity: usize,
}

impl Default for WasmLimits {
    fn default() -> Self {
        WasmConfig::default().limits()
    }
}

impl WasmLimits {
    #[must_use]
    pub fn memory_limit_bytes(&self) -> usize {
        usize::try_from(u64::from(self.memory_limit_mb) * 1024 * 1024).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProxyConfig;

    #[test]
    fn an_absent_wasm_section_uses_the_documented_defaults() {
        let config: ProxyConfig = toml::from_str("").unwrap();
        assert_eq!(config.wasm, WasmConfig::default());
        assert_eq!(config.wasm.epoch_tick, Duration::from_millis(50));
        assert_eq!(
            config.wasm.limits(),
            WasmLimits {
                memory_limit_mb: 64,
                cpu_budget: Duration::from_secs(3),
                codec_cpu_budget: Duration::from_millis(800),
                host_call_timeout: Duration::from_secs(30),
                max_call_duration: Duration::from_secs(60),
                queue_capacity: 1024,
            }
        );
    }

    #[test]
    fn the_wasm_section_parses_every_key() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm]
            epoch_tick = "20ms"
            memory_limit_mb = 128
            cpu_budget = "5s"
            codec_cpu_budget = "400ms"
            host_call_timeout = "10s"
            max_call_duration = "45s"
            queue_capacity = 64
            "#,
        )
        .unwrap();
        assert_eq!(config.wasm.epoch_tick, Duration::from_millis(20));
        assert_eq!(
            config.wasm.limits(),
            WasmLimits {
                memory_limit_mb: 128,
                cpu_budget: Duration::from_secs(5),
                codec_cpu_budget: Duration::from_millis(400),
                host_call_timeout: Duration::from_secs(10),
                max_call_duration: Duration::from_secs(45),
                queue_capacity: 64,
            }
        );
    }

    #[test]
    fn a_plugin_override_replaces_only_the_keys_it_sets() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm]
            memory_limit_mb = 128
            queue_capacity = 64

            [plugins.chatty.wasm]
            memory_limit_mb = 16
            cpu_budget = "500ms"
            "#,
        )
        .unwrap();
        let overrides = config.plugins["chatty"].wasm.as_ref();
        let limits = config.wasm.limits_for(overrides);
        assert_eq!(limits.memory_limit_mb, 16);
        assert_eq!(limits.cpu_budget, Duration::from_millis(500));
        assert_eq!(limits.queue_capacity, 64);
        assert_eq!(limits.host_call_timeout, Duration::from_secs(30));
        assert_eq!(config.wasm.limits_for(None), config.wasm.limits());
    }

    #[test]
    fn unknown_wasm_keys_are_rejected() {
        let global = toml::from_str::<ProxyConfig>("[wasm]\nmemory_limit = 64\n");
        assert!(global.is_err(), "a misspelt [wasm] key must not be ignored");
        let plugin = toml::from_str::<ProxyConfig>("[plugins.p.wasm]\nepoch_tick = \"10ms\"\n");
        assert!(
            plugin.is_err(),
            "epoch_tick is engine-wide and cannot be set per plugin"
        );
    }

    #[test]
    fn plugin_overrides_serialize_only_the_keys_that_are_set() {
        let overrides = PluginWasmConfig {
            memory_limit_mb: Some(16),
            max_call_duration: Some(Duration::from_secs(5)),
            ..PluginWasmConfig::default()
        };
        let text = toml::to_string(&overrides).unwrap();
        let back: PluginWasmConfig = toml::from_str(&text).unwrap();
        assert_eq!(back, overrides);
        assert!(!text.contains("cpu_budget"), "{text}");
    }

    #[test]
    fn memory_limit_converts_to_bytes() {
        let limits = WasmLimits {
            memory_limit_mb: 3,
            ..WasmLimits::default()
        };
        assert_eq!(limits.memory_limit_bytes(), 3 * 1024 * 1024);
    }
}
