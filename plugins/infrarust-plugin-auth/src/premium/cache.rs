//! In-memory cache for Mojang premium status lookups and failed auth tracking.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum PremiumStatus {
    Premium { mojang_uuid: Uuid },
    Cracked,
}

struct CacheEntry {
    status: PremiumStatus,
    cached_at: Instant,
}

pub struct PremiumCache {
    entries: DashMap<String, CacheEntry>,
    ttl: Duration,
    failed_auths: DashMap<String, Instant>,
    failed_auth_ttl: Duration,
}

impl PremiumCache {
    pub fn new(ttl: Duration, failed_auth_ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
            failed_auths: DashMap::new(),
            failed_auth_ttl,
        }
    }

    pub fn get(&self, username: &str) -> Option<PremiumStatus> {
        let key = username.to_lowercase();
        let entry = self.entries.get(&key)?;
        if entry.cached_at.elapsed() < self.ttl {
            Some(entry.status.clone())
        } else {
            drop(entry);
            self.entries.remove(&key);
            None
        }
    }

    pub fn put(&self, username: &str, status: PremiumStatus) {
        self.entries.insert(
            username.to_lowercase(),
            CacheEntry {
                status,
                cached_at: Instant::now(),
            },
        );
    }

    pub fn invalidate(&self, username: &str) {
        let key = username.to_lowercase();
        self.entries.remove(&key);
        self.failed_auths.remove(&key);
    }

    pub fn mark_auth_failed(&self, username: &str) {
        self.failed_auths
            .insert(username.to_lowercase(), Instant::now());
    }

    pub fn is_auth_failed(&self, username: &str) -> bool {
        let key = username.to_lowercase();
        let Some(entry) = self.failed_auths.get(&key) else {
            return false;
        };
        if entry.elapsed() < self.failed_auth_ttl {
            true
        } else {
            drop(entry);
            self.failed_auths.remove(&key);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn cache_hit_and_miss() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));

        assert!(cache.get("Steve").is_none());

        cache.put(
            "Steve",
            PremiumStatus::Premium {
                mojang_uuid: Uuid::nil(),
            },
        );
        let status = cache.get("steve").expect("should be cached");
        assert!(matches!(status, PremiumStatus::Premium { .. }));
    }

    #[test]
    fn cache_invalidation() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));
        cache.put("Steve", PremiumStatus::Cracked);

        cache.invalidate("Steve");
        assert!(cache.get("Steve").is_none());
    }

    #[test]
    fn cache_expiry() {
        let cache = PremiumCache::new(Duration::from_millis(0), Duration::from_secs(60));
        cache.put(
            "Steve",
            PremiumStatus::Premium {
                mojang_uuid: Uuid::nil(),
            },
        );

        assert!(cache.get("Steve").is_none());
    }

    #[test]
    fn failed_auth_remembered() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));

        assert!(!cache.is_auth_failed("Hypixel"));

        cache.mark_auth_failed("Hypixel");
        assert!(cache.is_auth_failed("hypixel"));
    }

    #[test]
    fn failed_auth_expires() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_millis(0));

        cache.mark_auth_failed("Hypixel");
        assert!(!cache.is_auth_failed("Hypixel"));
    }

    #[test]
    fn invalidate_clears_failed_auth() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));

        cache.mark_auth_failed("Steve");
        assert!(cache.is_auth_failed("Steve"));

        cache.invalidate("Steve");
        assert!(!cache.is_auth_failed("Steve"));
    }
}
