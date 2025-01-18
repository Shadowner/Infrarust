use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use tokio::{io, net::TcpStream, sync::Mutex};
use xxhash_rust::xxh64::xxh64;

use crate::Filter;

pub struct RateLimiter {
    request_limit: u32,
    counter: Arc<Mutex<LocalCounter>>,
    key_fn: Box<dyn Fn(&TcpStream) -> String + Send + Sync>,
}

impl RateLimiter {
    pub fn new(request_limit: u32, window_length: Duration) -> Self {
        Self {
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
            return Err(io::Error::new(io::ErrorKind::Other, "Rate limit exceeded"));
        }

        counter.increment(&key, now);
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
        // For IPv6, use /64 prefix
        let segments = ipv6.segments();
        format!(
            "{}:{}:{}:{}",
            segments[0], segments[1], segments[2], segments[3]
        )
    } else {
        ip.to_string()
    }
}

struct LocalCounter {
    counters: HashMap<u64, Count>,
    window_length: Duration,
    last_eviction: SystemTime,
}

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
        if now.duration_since(self.last_eviction).unwrap() < self.window_length {
            return;
        }

        self.counters
            .retain(|_, count| now.duration_since(count.timestamp).unwrap() < self.window_length);
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
            .unwrap()
            .as_secs()
            / self.window_length.as_secs();

        xxh64(format!("{}:{}", key, window).as_bytes(), 0)
    }
}

#[async_trait]
impl Filter for RateLimiter {
    async fn filter(&self, stream: &TcpStream) -> io::Result<()> {
        self.check_rate(stream).await
    }
}
