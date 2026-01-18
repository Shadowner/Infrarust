use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use infrarust_config::LogType;
use tokio::{io, net::TcpStream, sync::RwLock};
use tracing::debug;
use xxhash_rust::xxh64::Xxh64;

use crate::security::filter::{ConfigValue, Filter, FilterError, FilterType};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RateLimitKey {
    bytes: [u8; 8],
    len: u8,
}

impl RateLimitKey {
    pub fn from_socket_addr(addr: SocketAddr) -> Self {
        match addr.ip() {
            IpAddr::V4(ipv4) => {
                let mut bytes = [0u8; 8];
                bytes[..4].copy_from_slice(&ipv4.octets());
                Self { bytes, len: 4 }
            }
            IpAddr::V6(ipv6) => {
                let segments = ipv6.segments();
                let mut bytes = [0u8; 8];
                bytes[0..2].copy_from_slice(&segments[0].to_be_bytes());
                bytes[2..4].copy_from_slice(&segments[1].to_be_bytes());
                bytes[4..6].copy_from_slice(&segments[2].to_be_bytes());
                bytes[6..8].copy_from_slice(&segments[3].to_be_bytes());
                Self { bytes, len: 8 }
            }
        }
    }

    pub fn unknown() -> Self {
        Self {
            bytes: [0u8; 8],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

impl Hash for RateLimitKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl std::fmt::Debug for RateLimitKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.len == 0 {
            write!(f, "unknown")
        } else if self.len == 4 {
            write!(
                f,
                "{}.{}.{}.{}",
                self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]
            )
        } else {
            write!(
                f,
                "{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}",
                self.bytes[0],
                self.bytes[1],
                self.bytes[2],
                self.bytes[3],
                self.bytes[4],
                self.bytes[5],
                self.bytes[6],
                self.bytes[7]
            )
        }
    }
}

impl std::fmt::Display for RateLimitKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

pub struct RateLimiter {
    name: String,
    request_limit: u32,
    counter: Arc<RwLock<LocalCounter>>,
    key_fn: Box<dyn Fn(&TcpStream) -> RateLimitKey + Send + Sync>,
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
            counter: Arc::new(RwLock::new(LocalCounter::new(window_length))),
            key_fn: Box::new(key_by_ip),
        }
    }

    pub fn with_key_fn<F>(mut self, key_fn: F) -> Self
    where
        F: Fn(&TcpStream) -> RateLimitKey + Send + Sync + 'static,
    {
        self.key_fn = Box::new(key_fn);
        self
    }

    pub async fn check_rate(&self, stream: &TcpStream) -> io::Result<()> {
        let key = (self.key_fn)(stream);
        let now = SystemTime::now();

        let mut counter = self.counter.write().await;
        counter.evict();

        let rate = counter.get_rate(key, now);

        if rate >= f64::from(self.request_limit) {
            debug!(
                log_type = LogType::Filter.as_str(),
                "Rate limit exceeded for key: {}", key
            );
            return Err(io::Error::other("Rate limit exceeded"));
        }

        counter.increment(key, now);
        debug!(
            log_type = LogType::Filter.as_str(),
            "Rate check passed for key: {} (current rate: {}/{})", key, rate, self.request_limit
        );
        Ok(())
    }
}

fn key_by_ip(stream: &TcpStream) -> RateLimitKey {
    stream
        .peer_addr()
        .map(RateLimitKey::from_socket_addr)
        .unwrap_or_else(|_| RateLimitKey::unknown())
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

    fn get_rate(&self, key: RateLimitKey, now: SystemTime) -> f64 {
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

    fn increment(&mut self, key: RateLimitKey, now: SystemTime) {
        let hash = self.hash_key(key, now);
        self.counters
            .entry(hash)
            .and_modify(|c| c.value += 1)
            .or_insert(Count {
                value: 1,
                timestamp: now,
            });
    }

    fn hash_key(&self, key: RateLimitKey, time: SystemTime) -> u64 {
        let window = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.window_length.as_secs();

        // Use streaming hasher with zero-allocation key bytes
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::net::{TcpListener, TcpStream};

    async fn create_test_connection() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client_task = tokio::spawn(async move { TcpStream::connect(addr).await.unwrap() });

        let (server_stream, _) = listener.accept().await.unwrap();
        let client_stream = client_task.await.unwrap();

        (client_stream, server_stream)
    }

    #[tokio::test]
    async fn test_single_request_allowed() {
        let limiter = RateLimiter::new("test", 10, Duration::from_secs(60));
        let (client, _server) = create_test_connection().await;

        let result = limiter.check_rate(&client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_requests_within_limit() {
        let limiter = RateLimiter::new("test", 5, Duration::from_secs(60));
        let (client, _server) = create_test_connection().await;

        // Make 4 requests, all should pass (limit is 5)
        for i in 0..4 {
            let result = limiter.check_rate(&client).await;
            assert!(result.is_ok(), "Request {} should pass", i + 1);
        }
    }

    #[tokio::test]
    async fn test_requests_exceed_limit() {
        let limiter = RateLimiter::new("test", 3, Duration::from_secs(60));
        let (client, _server) = create_test_connection().await;

        // First 3 requests should pass
        for i in 0..3 {
            let result = limiter.check_rate(&client).await;
            assert!(result.is_ok(), "Request {} should pass", i + 1);
        }

        // 4th request should fail
        let result = limiter.check_rate(&client).await;
        assert!(result.is_err(), "Request 4 should be rate limited");
    }

    #[tokio::test]
    async fn test_different_ips_independent() {
        // Test that different IP keys are tracked independently in the counter
        let mut counter = LocalCounter::new(Duration::from_secs(60));
        let now = SystemTime::now();

        // Create keys for two different IPs
        let key1 = RateLimitKey::from_socket_addr("192.168.1.1:1234".parse().unwrap());
        let key2 = RateLimitKey::from_socket_addr("192.168.1.2:1234".parse().unwrap());

        // Increment key1 multiple times
        counter.increment(key1, now);
        counter.increment(key1, now);
        counter.increment(key1, now);

        // key1 should have rate of 3
        assert_eq!(counter.get_rate(key1, now), 3.0);

        // key2 should still have rate of 0 (independent tracking)
        assert_eq!(counter.get_rate(key2, now), 0.0);

        // Increment key2 once
        counter.increment(key2, now);
        assert_eq!(counter.get_rate(key2, now), 1.0);

        // key1's rate should be unchanged
        assert_eq!(counter.get_rate(key1, now), 3.0);
    }

    #[tokio::test]
    async fn test_rate_limit_key_from_ipv4() {
        let addr: SocketAddr = "192.168.1.100:12345".parse().unwrap();
        let key = RateLimitKey::from_socket_addr(addr);

        assert_eq!(key.len, 4);
        assert_eq!(format!("{}", key), "192.168.1.100");
    }

    #[tokio::test]
    async fn test_rate_limit_key_from_ipv6() {
        let addr: SocketAddr = "[2001:db8::1]:12345".parse().unwrap();
        let key = RateLimitKey::from_socket_addr(addr);

        assert_eq!(key.len, 8);
        // IPv6 keys use first 4 segments
    }

    #[tokio::test]
    async fn test_rate_limit_key_unknown() {
        let key = RateLimitKey::unknown();
        assert_eq!(key.len, 0);
        assert_eq!(format!("{}", key), "unknown");
    }

    #[tokio::test]
    async fn test_local_counter_increment_and_get() {
        let mut counter = LocalCounter::new(Duration::from_secs(60));
        let key = RateLimitKey::from_socket_addr("192.168.1.1:1234".parse().unwrap());
        let now = SystemTime::now();

        // Initially rate should be 0
        assert_eq!(counter.get_rate(key, now), 0.0);

        // After increment, rate should be 1
        counter.increment(key, now);
        assert_eq!(counter.get_rate(key, now), 1.0);

        // After another increment, rate should be 2
        counter.increment(key, now);
        assert_eq!(counter.get_rate(key, now), 2.0);
    }

    #[tokio::test]
    async fn test_local_counter_evict() {
        let mut counter = LocalCounter::new(Duration::from_secs(1));
        let key = RateLimitKey::from_socket_addr("192.168.1.1:1234".parse().unwrap());
        let now = SystemTime::now();

        counter.increment(key, now);
        assert_eq!(counter.counters.len(), 1);

        // Evict should not remove entries within window
        counter.evict();
        assert!(!counter.counters.is_empty() || counter.last_eviction == now);
    }

    #[tokio::test]
    async fn test_rate_limiter_filter_trait() {
        let limiter = RateLimiter::new("test_filter", 10, Duration::from_secs(60));

        assert_eq!(limiter.name(), "test_filter");
        assert_eq!(limiter.filter_type(), FilterType::RateLimiter);
        assert!(limiter.is_configurable());
    }
}
