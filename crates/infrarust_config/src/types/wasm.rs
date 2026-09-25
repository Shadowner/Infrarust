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

    #[serde(default)]
    pub recovery: WasmRecoveryConfig,
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
            recovery: WasmRecoveryConfig::default(),
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
            recovery: self.recovery,
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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<PluginWasmRecoveryConfig>,
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
            recovery: self
                .recovery
                .as_ref()
                .map_or(base.recovery, |overrides| overrides.apply(base.recovery)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WasmRecoveryConfig {
    #[serde(default = "defaults::wasm_recovery_max_restarts")]
    pub max_restarts: u32,

    #[serde(default = "defaults::wasm_recovery_window")]
    #[serde(with = "humantime_serde")]
    pub window: Duration,

    #[serde(default = "defaults::wasm_recovery_backoff_initial")]
    #[serde(with = "humantime_serde")]
    pub backoff_initial: Duration,

    #[serde(default = "defaults::wasm_recovery_backoff_max")]
    #[serde(with = "humantime_serde")]
    pub backoff_max: Duration,
}

impl Default for WasmRecoveryConfig {
    fn default() -> Self {
        Self {
            max_restarts: defaults::wasm_recovery_max_restarts(),
            window: defaults::wasm_recovery_window(),
            backoff_initial: defaults::wasm_recovery_backoff_initial(),
            backoff_max: defaults::wasm_recovery_backoff_max(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginWasmRecoveryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_restarts: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub window: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub backoff_initial: Option<Duration>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "humantime_serde::option")]
    pub backoff_max: Option<Duration>,
}

impl PluginWasmRecoveryConfig {
    #[must_use]
    pub fn apply(&self, base: WasmRecoveryConfig) -> WasmRecoveryConfig {
        WasmRecoveryConfig {
            max_restarts: self.max_restarts.unwrap_or(base.max_restarts),
            window: self.window.unwrap_or(base.window),
            backoff_initial: self.backoff_initial.unwrap_or(base.backoff_initial),
            backoff_max: self.backoff_max.unwrap_or(base.backoff_max),
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
    pub recovery: WasmRecoveryConfig,
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
                recovery: WasmRecoveryConfig::default(),
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
                recovery: WasmRecoveryConfig::default(),
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
    fn an_absent_recovery_table_uses_the_documented_defaults() {
        let config: ProxyConfig = toml::from_str("[wasm]\nqueue_capacity = 8\n").unwrap();
        let recovery = config.wasm.limits().recovery;
        assert_eq!(recovery.max_restarts, 5);
        assert_eq!(recovery.window, Duration::from_secs(300));
        assert_eq!(recovery.backoff_initial, Duration::from_secs(1));
        assert_eq!(recovery.backoff_max, Duration::from_secs(300));
    }

    #[test]
    fn the_recovery_table_parses_and_a_plugin_overrides_only_its_keys() {
        let config: ProxyConfig = toml::from_str(
            r#"
            [wasm.recovery]
            max_restarts = 3
            window = "1m"
            backoff_initial = "2s"
            backoff_max = "10m"

            [plugins.flaky.wasm.recovery]
            max_restarts = 1
            "#,
        )
        .unwrap();
        assert_eq!(
            config.wasm.limits().recovery,
            WasmRecoveryConfig {
                max_restarts: 3,
                window: Duration::from_secs(60),
                backoff_initial: Duration::from_secs(2),
                backoff_max: Duration::from_secs(600),
            }
        );
        let flaky = config
            .wasm
            .limits_for(config.plugins["flaky"].wasm.as_ref())
            .recovery;
        assert_eq!(flaky.max_restarts, 1);
        assert_eq!(flaky.window, Duration::from_secs(60));
        assert_eq!(flaky.backoff_max, Duration::from_secs(600));
    }

    #[test]
    fn unknown_recovery_keys_are_rejected() {
        let global = toml::from_str::<ProxyConfig>("[wasm.recovery]\nmax_retries = 3\n");
        assert!(
            global.is_err(),
            "a misspelt recovery key must not be ignored"
        );
        let plugin = toml::from_str::<ProxyConfig>("[plugins.p.wasm.recovery]\nwindows = \"1m\"\n");
        assert!(plugin.is_err());
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
