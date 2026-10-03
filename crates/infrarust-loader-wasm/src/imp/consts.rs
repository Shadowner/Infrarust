use std::time::Duration;

pub(crate) const EPOCH_DEADLINE_TICKS: u64 = 1;

pub(crate) const LEGACY_CACHE_SUBDIR: &str = ".cache";

pub(crate) const MAX_COMPONENT_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) const ERROR_TEXT_LIMIT: usize = 1024;

pub(crate) use infrarust_plugin_wit::WORLD_VERSION;

pub(crate) const PLAYER_SWITCH_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) const MAX_BOSS_BARS: usize = 256;

pub(crate) const MAX_HOST_HANDLES: usize = 4096;

pub(crate) const QUEUE_FULL_WARN_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) const DEADLINE_MARGIN_DIVISOR: u32 = 5;

pub(crate) const MAX_DEADLINE_MARGIN: Duration = Duration::from_millis(250);

pub(crate) const FAR_FUTURE: Duration = Duration::from_secs(86_400 * 365 * 30);

pub(crate) const DENIED_CALL_LOG_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const COMMAND_REFUSAL_BURST: u32 = 5;
pub(crate) const CODEC_REFUSAL_BURST: u32 = 5;

pub(crate) const CODEC_LOG_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) const CODEC_LOG_BURST: u32 = 20;

pub(crate) const GUEST_WARNING_INTERVAL: Duration = Duration::from_secs(60);

pub(crate) const GUEST_WARNING_BURST: u32 = 5;
