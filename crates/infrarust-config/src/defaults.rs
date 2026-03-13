//! Default values for configuration fields.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub fn bind() -> SocketAddr {
    "0.0.0.0:25565".parse().unwrap()
}

pub fn connect_timeout() -> Duration {
    Duration::from_secs(5)
}

pub fn servers_dir() -> PathBuf {
    PathBuf::from("./servers")
}

pub fn rate_limit_max() -> u32 {
    3
}
pub fn rate_limit_window() -> Duration {
    Duration::from_secs(10)
}
pub fn rate_limit_status_max() -> u32 {
    30
}
pub fn rate_limit_status_window() -> Duration {
    Duration::from_secs(10)
}

pub fn status_cache_ttl() -> Duration {
    Duration::from_secs(5)
}
pub fn status_cache_max_entries() -> usize {
    1000
}

pub fn read_timeout() -> Duration {
    Duration::from_secs(30)
}
pub fn write_timeout() -> Duration {
    Duration::from_secs(30)
}

pub fn ready_pattern() -> String {
    r#"For help, type "help""#.to_string()
}

pub fn shutdown_timeout() -> Duration {
    Duration::from_secs(30)
}

pub fn otlp_endpoint() -> String {
    "http://localhost:4317".to_string()
}

pub fn service_name() -> String {
    "infrarust".to_string()
}
