//! Default values for configuration fields.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub fn bind() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], 25565))
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

pub const fn rate_limit_max() -> u32 {
    3
}
pub const fn rate_limit_window() -> Duration {
    Duration::from_secs(10)
}
pub const fn rate_limit_status_max() -> u32 {
    30
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

pub fn ban_file() -> std::path::PathBuf {
    std::path::PathBuf::from("bans.json")
}

pub const fn ban_purge_interval() -> std::time::Duration {
    std::time::Duration::from_secs(300)
}

pub const fn ban_audit_log() -> bool {
    true
}

pub fn docker_endpoint() -> String {
    "unix:///var/run/docker.sock".to_string()
}

pub const fn docker_poll_interval() -> std::time::Duration {
    std::time::Duration::from_secs(30)
}

pub const fn docker_reconnect_delay() -> std::time::Duration {
    std::time::Duration::from_secs(5)
}

pub const fn announce_proxy_commands() -> bool {
    true
}
