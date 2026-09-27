//! Error types for configuration handling.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use humantime::format_duration;

use crate::migrate::MigrationWarning;
use crate::types::ProxyMode;
use crate::validation::{
    MAX_IDENTIFIER_LEN, WASM_MAX_DURATION, WASM_MAX_EPOCH_TICK, WASM_MAX_INSTANCE_POOL,
    WASM_MAX_MEMORY_MB, WASM_MAX_QUEUE_CAPACITY, WASM_MAX_QUOTA, WASM_MAX_RESTARTS,
    WASM_MIN_EPOCH_TICK,
};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to read directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse TOML in {path}: {source}")]
    ParseToml {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("failed to parse YAML in {path}: {source}")]
    ParseYaml {
        path: PathBuf,
        source: serde_yml::Error,
    },

    #[error("failed to serialize TOML for {path}: {source}")]
    SerializeToml {
        path: PathBuf,
        source: toml::ser::Error,
    },

    #[error("invalid server address: {0}")]
    InvalidAddress(String),

    #[error(
        "server '{id}' uses {proxy_mode:?} mode which requires at least one domain \
             (forwarding modes are only accessible via direct domain connection)"
    )]
    NoDomains { id: String, proxy_mode: ProxyMode },

    #[error("server config {id} has no addresses defined")]
    NoAddresses { id: String },

    #[error("duplicate config id: {0}")]
    DuplicateId(String),

    #[error("config directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("server '{id}': {reason}")]
    Server {
        id: String,
        reason: ServerValidationError,
    },

    #[error(transparent)]
    Proxy(ProxyValidationError),

    #[error(transparent)]
    Wasm(WasmValidationError),

    #[error(
        "migration produced no output: all {skipped} candidate file(s) were skipped — {}",
        describe_warnings(.warnings)
    )]
    NothingMigrated {
        skipped: usize,
        warnings: Vec<MigrationWarning>,
    },
}

fn describe_warnings(warnings: &[MigrationWarning]) -> String {
    warnings
        .iter()
        .map(|w| format!("{}: {}", w.file, w.message))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ServerValidationError {
    #[error(
        "{proxy_mode:?} mode cannot belong to a network \
         (forwarding modes don't support server switching)"
    )]
    ForwardingInNetwork { proxy_mode: ProxyMode },

    #[error(
        "{proxy_mode:?} mode does not read the backend's packets: \
         bungeecord_channel needs offline or client_only"
    )]
    BungeecordChannelNotIntercepted { proxy_mode: ProxyMode },

    #[error(
        "proxy_mode = \"full\" is reserved and not implemented; use \
         client_only, offline, passthrough, zero_copy or server_only"
    )]
    FullModeReserved,

    #[error(
        "{proxy_mode:?} mode does not parse the login, but {mode_source} is velocity: \
         velocity forwarding needs offline or client_only, or set \
         forwarding_mode = \"none\", \"bungeecord\" or \"bungeeguard\" on the server"
    )]
    VelocityOnForwardingServer {
        proxy_mode: ProxyMode,
        mode_source: &'static str,
    },

    #[error("domains must not contain an empty entry")]
    EmptyDomain,

    #[error("slow_start_aggression must be a finite number > 0 (got {value})")]
    SlowStartAggression { value: f64 },

    #[error("{field} must not be empty")]
    EmptyIdentifier { field: &'static str },

    #[error("{field} must be at most {MAX_IDENTIFIER_LEN} characters")]
    IdentifierTooLong { field: &'static str },

    #[error("{field} '{value}' contains invalid characters (allowed: {allowed})")]
    IdentifierChars {
        field: &'static str,
        value: String,
        allowed: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProxyValidationError {
    #[error("{key} must be greater than zero")]
    ZeroDuration { key: &'static str },

    #[error(
        "web.enable_webui requires web.enable_api: the dashboard is served by the \
         admin API and cannot run without it"
    )]
    WebUiWithoutApi,

    #[error("{0}")]
    WebApiKey(String),

    #[error("web.bind '{bind}' is not a valid bind address (expected host:port)")]
    WebBindNotHostPort { bind: String },

    #[error("web.bind '{bind}' has an invalid port '{port}'")]
    WebBindInvalidPort { bind: String, port: String },

    #[error("web.bind '{bind}' has an invalid host")]
    WebBindInvalidHost { bind: String },

    #[error("web.bind '{bind}' collides with the proxy bind address '{proxy_bind}'")]
    WebBindCollision {
        bind: String,
        proxy_bind: SocketAddr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WasmValidationError {
    #[error(
        "wasm.epoch_tick must be between {} and {} (got {})",
        format_duration(WASM_MIN_EPOCH_TICK),
        format_duration(WASM_MAX_EPOCH_TICK),
        format_duration(*.tick)
    )]
    EpochTickOutOfRange { tick: Duration },

    #[error("wasm.instance_pool must be at most {WASM_MAX_INSTANCE_POOL} (got {value})")]
    InstancePoolTooLarge { value: u32 },

    #[error("{scope}.memory_limit_mb must be between 1 and {WASM_MAX_MEMORY_MB} (got {value})")]
    MemoryLimitOutOfRange { scope: String, value: u32 },

    #[error(
        "{scope}.{key} must be between wasm.epoch_tick ({}) and {} (got {})",
        format_duration(*.tick),
        format_duration(WASM_MAX_DURATION),
        format_duration(*.value)
    )]
    CpuBudgetOutOfRange {
        scope: String,
        key: &'static str,
        tick: Duration,
        value: Duration,
    },

    #[error(
        "{scope}.{key} must be greater than zero and at most {} (got {})",
        format_duration(*.max),
        format_duration(*.value)
    )]
    DurationOutOfRange {
        scope: String,
        key: &'static str,
        max: Duration,
        value: Duration,
    },

    #[error("{scope}.queue_capacity must be between 1 and {WASM_MAX_QUEUE_CAPACITY} (got {value})")]
    QueueCapacityOutOfRange { scope: String, value: usize },

    #[error("{scope}.recovery.max_restarts must be at most {WASM_MAX_RESTARTS} (got {value})")]
    MaxRestartsTooLarge { scope: String, value: u32 },

    #[error("{scope}.quotas.{key} must be between 1 and {WASM_MAX_QUOTA} (got {value})")]
    QuotaOutOfRange {
        scope: String,
        key: &'static str,
        value: usize,
    },

    #[error(
        "{scope}.recovery.backoff_initial ({}) must not be longer than {scope}.recovery.backoff_max ({})",
        format_duration(*.initial),
        format_duration(*.max)
    )]
    BackoffInitialExceedsMax {
        scope: String,
        initial: Duration,
        max: Duration,
    },

    #[error("{scope}: the host path of \"{guest}\" must not be empty")]
    MountHostEmpty { scope: String, guest: String },

    #[error("{scope}: {reason}")]
    MountGuestPath { scope: String, reason: String },

    #[error("{scope}: guest path \"{guest}\" is mounted twice")]
    MountDuplicate { scope: String, guest: String },

    #[error("{scope}: guest paths \"{short}\" and \"{long}\" overlap")]
    MountOverlap {
        scope: String,
        short: String,
        long: String,
    },
}
