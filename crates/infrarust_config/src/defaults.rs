//! Default values for configuration fields.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub fn bind() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 25565))
}

pub const fn connect_max_attempts() -> usize {
    3
}

pub const fn connect_timeout() -> Duration {
    Duration::from_secs(5)
}

pub fn servers_dir() -> PathBuf {
    PathBuf::from("./servers")
}

pub fn plugins_dir() -> PathBuf {
    PathBuf::from("./plugins")
}

pub const fn rate_limit_enabled() -> bool {
    false
}
pub const fn rate_limit_max() -> u32 {
    3
}
pub const fn rate_limit_window() -> Duration {
    Duration::from_secs(10)
}
pub const fn rate_limit_status_max() -> u32 {
    300
}
pub const fn rate_limit_status_window() -> Duration {
    Duration::from_secs(10)
}

pub const fn status_cache_ttl() -> Duration {
    Duration::from_secs(5)
}
pub const fn status_cache_max_entries() -> usize {
    1000
}

pub const fn read_timeout() -> Duration {
    Duration::from_secs(30)
}
pub const fn write_timeout() -> Duration {
    Duration::from_secs(30)
}

pub fn ready_pattern() -> String {
    r#"For help, type "help""#.to_string()
}

pub const fn shutdown_timeout() -> Duration {
    Duration::from_secs(30)
}

pub const fn start_timeout() -> Duration {
    Duration::from_secs(60)
}

pub const fn poll_interval() -> Duration {
    Duration::from_secs(5)
}

pub fn otlp_endpoint() -> String {
    "http://localhost:4317".to_string()
}

pub fn service_name() -> String {
    "infrarust".to_string()
}

pub fn telemetry_protocol() -> String {
    "grpc".to_string()
}

pub const fn true_val() -> bool {
    true
}

pub const fn metrics_export_interval() -> Duration {
    Duration::from_secs(15)
}

pub const fn sampling_ratio() -> f64 {
    0.1
}

pub fn service_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub const fn keepalive_time() -> Duration {
    Duration::from_secs(30)
}

pub const fn keepalive_interval() -> Duration {
    Duration::from_secs(10)
}

pub const fn keepalive_retries() -> u32 {
    3
}

pub fn ban_file() -> PathBuf {
    PathBuf::from("bans.json")
}

pub const fn ban_purge_interval() -> Duration {
    Duration::from_secs(300)
}

pub const fn ban_audit_log() -> bool {
    true
}

pub const fn ban_check_timeout() -> Duration {
    Duration::from_secs(5)
}

pub fn docker_endpoint() -> String {
    "unix:///var/run/docker.sock".to_string()
}

pub const fn docker_poll_interval() -> Duration {
    Duration::from_secs(30)
}

pub const fn docker_reconnect_delay() -> Duration {
    Duration::from_secs(5)
}

pub const fn announce_proxy_commands() -> bool {
    true
}

pub fn session_url() -> String {
    "https://sessionserver.mojang.com/session/minecraft/hasJoined".to_string()
}

pub const fn event_handler_timeout() -> Duration {
    Duration::from_secs(10)
}

pub const fn event_slow_handler_threshold() -> Duration {
    Duration::from_secs(1)
}

pub const fn event_packet_handler_timeout() -> Duration {
    Duration::from_secs(10)
}

pub const fn event_disconnect_deadline() -> Duration {
    Duration::from_secs(15)
}

pub const fn event_transport_filter_timeout() -> Duration {
    Duration::from_secs(5)
}

pub const fn wasm_epoch_tick() -> Duration {
    Duration::from_millis(50)
}

pub const fn wasm_memory_limit_mb() -> u32 {
    64
}

pub const fn wasm_cpu_budget() -> Duration {
    Duration::from_secs(3)
}

pub const fn wasm_codec_cpu_budget() -> Duration {
    Duration::from_millis(800)
}

pub const fn wasm_host_call_timeout() -> Duration {
    Duration::from_secs(30)
}

pub const fn wasm_max_call_duration() -> Duration {
    Duration::from_secs(60)
}

pub const fn wasm_queue_capacity() -> usize {
    1024
}

pub const fn wasm_instance_pool() -> u32 {
    0
}

pub const fn wasm_recovery_max_restarts() -> u32 {
    5
}

pub const fn wasm_recovery_window() -> Duration {
    Duration::from_secs(300)
}

pub const fn wasm_recovery_backoff_initial() -> Duration {
    Duration::from_secs(1)
}

pub const fn wasm_recovery_backoff_max() -> Duration {
    Duration::from_secs(300)
}

pub const fn wasm_network_http() -> bool {
    true
}

pub const fn wasm_mount_read_only() -> bool {
    true
}
