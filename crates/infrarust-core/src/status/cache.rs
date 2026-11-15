//! TTL-based status response cache.
//!
//! Stores the last known `ServerPingResponse` per server, with fresh/stale
//! semantics. Fresh entries are served immediately; stale entries act as
//! fallback when the backend is unreachable.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use infrarust_protocol::LegacyPingResponse;
use infrarust_protocol::legacy::LegacyPingVariant;

use super::response::ServerPingResponse;

/// Cache of status responses keyed by server id.
///
/// Entries are bounded by the number of configured servers (typically < 50).
/// No background cleanup — expiry is checked lazily on `get_fresh()`.
pub struct StatusCache {
    entries: DashMap<String, CachedStatus>,
    legacy_entries: DashMap<(String, LegacyPingVariant), CachedLegacy>,
    default_ttl: Duration,
}

struct CachedLegacy {
    response: LegacyPingResponse,
    fetched_at: Instant,
}

struct CachedStatus {
    response: ServerPingResponse,
    latency: Duration,
    fetched_at: Instant,
    ttl: Duration,
}

impl StatusCache {
    /// Creates a new cache with the given default TTL.
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            legacy_entries: DashMap::new(),
            default_ttl,
        }
    }

    /// Returns the cached response if the TTL has not expired.
    pub fn get_fresh(&self, server_id: &str) -> Option<(ServerPingResponse, Duration)> {
        let entry = self.entries.get(server_id)?;
        if entry.fetched_at.elapsed() < entry.ttl {
            Some((entry.response.clone(), entry.latency))
        } else {
            None
        }
    }

    /// Returns the cached response regardless of TTL (stale fallback).
    pub fn get_stale(&self, server_id: &str) -> Option<(ServerPingResponse, Duration)> {
        let entry = self.entries.get(server_id)?;
        Some((entry.response.clone(), entry.latency))
    }

    /// Inserts or replaces a cache entry.
    ///
    /// If `ttl` is `None`, uses the default TTL.
    pub fn put(
        &self,
        server_id: &str,
        response: ServerPingResponse,
        latency: Duration,
        ttl: Option<Duration>,
    ) {
        self.entries.insert(
            server_id.to_string(),
            CachedStatus {
                response,
                latency,
                fetched_at: Instant::now(),
                ttl: ttl.unwrap_or(self.default_ttl),
            },
        );
    }

    /// Returns the reply a backend gave to a legacy ping if the TTL has not expired.
    pub fn get_fresh_legacy(
        &self,
        server_id: &str,
        variant: LegacyPingVariant,
    ) -> Option<LegacyPingResponse> {
        let entry = self.legacy_entries.get(&(server_id.to_string(), variant))?;
        (entry.fetched_at.elapsed() < self.default_ttl).then(|| entry.response.clone())
    }

    pub fn put_legacy(
        &self,
        server_id: &str,
        variant: LegacyPingVariant,
        response: LegacyPingResponse,
    ) {
        self.legacy_entries.insert(
            (server_id.to_string(), variant),
            CachedLegacy {
                response,
                fetched_at: Instant::now(),
            },
        );
    }

    /// Removes a single server's cache entries.
    pub fn invalidate(&self, server_id: &str) {
        self.entries.remove(server_id);
        self.legacy_entries.retain(|(id, _), _| id != server_id);
    }

    /// Removes all cache entries.
    pub fn invalidate_all(&self) {
        self.entries.clear();
        self.legacy_entries.clear();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use infrarust_protocol::ProtocolVersion;
    use infrarust_protocol::legacy::LegacyPingVariant;

    use super::*;

    fn sample_response() -> ServerPingResponse {
        ServerPingResponse::synthetic("Test", ProtocolVersion::V1_21, Some(100))
    }

    #[test]
    fn test_put_and_get_fresh() {
        let cache = StatusCache::new(Duration::from_secs(5));
        cache.put("srv", sample_response(), Duration::from_millis(10), None);

        let (resp, latency) = cache.get_fresh("srv").expect("should be fresh");
        assert_eq!(resp.players.max, 100);
        assert_eq!(latency, Duration::from_millis(10));
    }

    #[test]
    fn test_get_fresh_missing() {
        let cache = StatusCache::new(Duration::from_secs(5));
        assert!(cache.get_fresh("missing").is_none());
    }

    #[test]
    fn test_ttl_expired_fresh_none() {
        let cache = StatusCache::new(Duration::from_millis(1));
        cache.put("srv", sample_response(), Duration::ZERO, None);
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_fresh("srv").is_none());
    }

    #[test]
    fn test_ttl_expired_stale_some() {
        let cache = StatusCache::new(Duration::from_millis(1));
        cache.put("srv", sample_response(), Duration::ZERO, None);
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_fresh("srv").is_none());
        let (resp, _) = cache.get_stale("srv").expect("stale should exist");
        assert_eq!(resp.players.max, 100);
    }

    #[test]
    fn test_legacy_entries_expire_and_are_invalidated_with_the_rest() {
        let reply = || LegacyPingResponse {
            protocol_version: 78,
            server_version: "1.6.4".to_string(),
            motd: "Test".to_string(),
            online_players: 1,
            max_players: 2,
        };
        let cache = StatusCache::new(Duration::from_secs(60));
        cache.put_legacy("srv", LegacyPingVariant::V1_6, reply());
        assert_eq!(
            cache
                .get_fresh_legacy("srv", LegacyPingVariant::V1_6)
                .unwrap()
                .max_players,
            2
        );
        cache.invalidate_all();
        assert!(
            cache
                .get_fresh_legacy("srv", LegacyPingVariant::V1_6)
                .is_none()
        );

        let expired = StatusCache::new(Duration::from_millis(1));
        expired.put_legacy("srv", LegacyPingVariant::V1_6, reply());
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            expired
                .get_fresh_legacy("srv", LegacyPingVariant::V1_6)
                .is_none()
        );
    }

    #[test]
    fn test_legacy_entries_are_kept_per_ping_variant() {
        let reply = |protocol_version| LegacyPingResponse {
            protocol_version,
            server_version: String::new(),
            motd: "Test".to_string(),
            online_players: 1,
            max_players: 2,
        };
        let cache = StatusCache::new(Duration::from_secs(60));
        cache.put_legacy("srv", LegacyPingVariant::Beta, reply(0));
        assert!(
            cache
                .get_fresh_legacy("srv", LegacyPingVariant::V1_6)
                .is_none()
        );

        cache.put_legacy("srv", LegacyPingVariant::V1_6, reply(78));
        cache.put_legacy("other", LegacyPingVariant::V1_6, reply(78));
        let protocol = |variant| {
            cache
                .get_fresh_legacy("srv", variant)
                .map(|cached| cached.protocol_version)
        };
        assert_eq!(protocol(LegacyPingVariant::Beta), Some(0));
        assert_eq!(protocol(LegacyPingVariant::V1_6), Some(78));
        assert_eq!(protocol(LegacyPingVariant::V1_4), None);

        cache.invalidate("srv");
        assert_eq!(protocol(LegacyPingVariant::Beta), None);
        assert_eq!(protocol(LegacyPingVariant::V1_6), None);
        assert!(
            cache
                .get_fresh_legacy("other", LegacyPingVariant::V1_6)
                .is_some()
        );
    }

    #[test]
    fn test_invalidate_server() {
        let cache = StatusCache::new(Duration::from_secs(60));
        cache.put("srv", sample_response(), Duration::ZERO, None);
        assert!(cache.get_fresh("srv").is_some());
        cache.invalidate("srv");
        assert!(cache.get_fresh("srv").is_none());
        assert!(cache.get_stale("srv").is_none());
    }

    #[test]
    fn test_invalidate_all() {
        let cache = StatusCache::new(Duration::from_secs(60));
        cache.put("a", sample_response(), Duration::ZERO, None);
        cache.put("b", sample_response(), Duration::ZERO, None);
        cache.invalidate_all();
        assert!(cache.get_fresh("a").is_none());
        assert!(cache.get_fresh("b").is_none());
    }

    #[test]
    fn test_custom_ttl_per_entry() {
        let cache = StatusCache::new(Duration::from_secs(60));
        // Entry with a 1ms TTL
        cache.put(
            "short",
            sample_response(),
            Duration::ZERO,
            Some(Duration::from_millis(1)),
        );
        std::thread::sleep(Duration::from_millis(5));
        assert!(cache.get_fresh("short").is_none());
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        let cache = Arc::new(StatusCache::new(Duration::from_secs(5)));
        let mut handles = Vec::new();

        for i in 0..100 {
            let cache = Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                let key = format!("srv-{}", i % 10);
                cache.put(&key, sample_response(), Duration::ZERO, None);
                cache.get_fresh(&key);
                cache.get_stale(&key);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
