use std::time::Duration;

pub(crate) const EPOCH_DEADLINE_TICKS: u64 = 1;

pub(crate) const CACHE_SUBDIR: &str = ".cache";

pub(crate) const WASMTIME_CACHE_TAG: &str = "wasmtime-45";

pub(crate) use infrarust_plugin_wit::WORLD_VERSION;

pub(crate) const PLAYER_SWITCH_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) const QUEUE_FULL_WARN_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) const DEADLINE_MARGIN_DIVISOR: u32 = 5;

pub(crate) const MAX_DEADLINE_MARGIN: Duration = Duration::from_millis(250);

pub(crate) const FAR_FUTURE: Duration = Duration::from_secs(86_400 * 365 * 30);
