use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use infrarust_config::LogType;
use tokio::{io, net::TcpStream, sync::Mutex};
use tracing::debug;
use xxhash_rust::xxh64::Xxh64;

use crate::security::filter::{ConfigValue, Filter, FilterError, FilterType};

pub struct RateLimiter {
    name: String,
    request_limit: u32,
    counter: Arc<Mutex<LocalCounter>>,
    key_fn: Box<dyn Fn(&TcpStream) -> String + Send + Sync>,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("name", &self.name)
            .field("request_limit", &self.request_limit)
            .field("counter", &self.counter)
            .field("key_fn", &"<function>")
            .finish()
    }
}

impl RateLimiter {
    pub fn new(name: impl Into<String>, request_limit: u32, window_length: Duration) -> Self {
        let name_str = name.into();
        debug!(
            log_type = LogType::Filter.as_str(),
            "Creating new rate limiter: {}", name_str
        );
        Self {
            name: name_str,
            request_limit,
            counter: Arc::new(Mutex::new(LocalCounter::new(window_length))),
            key_fn: Box::new(key_by_ip),
        }
    }

    pub fn with_key_fn<F>(mut self, key_fn: F) -> Self
    where
        F: Fn(&TcpStream) -> String + Send + Sync + 'static,
    {
        self.key_fn = Box::new(key_fn);
        self
    }

    pub async fn check_rate(&self, stream: &TcpStream) -> io::Result<()> {
        let key = (self.key_fn)(stream);
        let now = SystemTime::now();

        let mut counter = self.counter.lock().await;
        counter.evict();

        let rate = counter.get_rate(&key, now);

        if rate >= f64::from(self.request_limit) {
            debug!(
                log_type = LogType::Filter.as_str(),
                "Rate limit exceeded for key: {}", key
            );
            return Err(io::Error::other("Rate limit exceeded"));
        }

        counter.increment(&key, now);
        debug!(
            log_type = LogType::Filter.as_str(),
            "Rate check passed for key: {} (current rate: {}/{})", key, rate, self.request_limit
        );
        Ok(())
    }
}

fn key_by_ip(stream: &TcpStream) -> String {
    stream
        .peer_addr()
        .map(canonicalize_ip_addr)
        .unwrap_or_else(|_| "unknown".to_string())
}

fn canonicalize_ip_addr(addr: SocketAddr) -> String {
    let ip = addr.ip();
    if ip.is_ipv4() {
        ip.to_string()
    } else if let std::net::IpAddr::V6(ipv6) = ip {
        let segments = ipv6.segments();
        format!(
            "{}:{}:{}:{}",
            segments[0], segments[1], segments[2], segments[3]
        )
    } else {
        ip.to_string()
    }
}
#[derive(Debug)]
struct LocalCounter {
    counters: HashMap<u64, Count>,
    window_length: Duration,
    last_eviction: SystemTime,
}

#[derive(Debug)]
struct Count {
    value: u32,
    timestamp: SystemTime,
}

impl LocalCounter {
    fn new(window_length: Duration) -> Self {
        Self {
            counters: HashMap::new(),
            window_length,
            last_eviction: SystemTime::now(),
        }
    }

    fn evict(&mut self) {
        let now = SystemTime::now();
        let since_last = now
            .duration_since(self.last_eviction)
            .unwrap_or(Duration::ZERO);

        if since_last < self.window_length {
            return;
        }

        self.counters.retain(|_, count| {
            now.duration_since(count.timestamp)
                .unwrap_or(Duration::MAX)
                < self.window_length
        });
        self.last_eviction = now;
    }

    fn get_rate(&self, key: &str, now: SystemTime) -> f64 {
        let hash = self.hash_key(key, now);
        let prev_hash = self.hash_key(key, now - self.window_length);

        let current = self.counters.get(&hash).map_or(0, |c| c.value);
        let previous = self.counters.get(&prev_hash).map_or(0, |c| c.value);

        let elapsed = now
            .duration_since(now - self.window_length)
            .unwrap_or(Duration::from_secs(0));

        f64::from(previous) * (1.0 - elapsed.as_secs_f64() / self.window_length.as_secs_f64())
            + f64::from(current)
    }

    fn increment(&mut self, key: &str, now: SystemTime) {
        let hash = self.hash_key(key, now);
        self.counters
            .entry(hash)
            .and_modify(|c| c.value += 1)
            .or_insert(Count {
                value: 1,
                timestamp: now,
            });
    }

    fn hash_key(&self, key: &str, time: SystemTime) -> u64 {
        let window = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.window_length.as_secs();

        // Use streaming hasher to avoid string allocation
        let mut hasher = Xxh64::new(0);
        hasher.update(key.as_bytes());
        hasher.update(&window.to_le_bytes());
        hasher.digest()
    }
}

#[async_trait]
impl Filter for RateLimiter {
    async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        self.check_rate(stream).await
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn filter_type(&self) -> FilterType {
        FilterType::RateLimiter
    }

    fn is_configurable(&self) -> bool {
        true
    }

    async fn apply_config(&self, config: ConfigValue) -> Result<(), FilterError> {
        if let ConfigValue::Map(_) = config {
            // We could update request_limit or window_length here
            Ok(())
        } else {
            Err(FilterError::InvalidConfig(
                "Expected a map configuration".to_string(),
            ))
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
