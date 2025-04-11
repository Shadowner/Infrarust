use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub status_ttl_seconds: Option<u64>,
    pub max_status_entries: Option<usize>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            status_ttl_seconds: Some(30),
            max_status_entries: Some(1000),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusCacheOptions {
    pub enabled: bool,
    pub ttl: Duration,
    pub max_size: usize,
}