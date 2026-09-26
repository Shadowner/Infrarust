//! In-memory cache for Mojang premium status lookups and failed auth tracking.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum PremiumStatus {
    Premium { mojang_uuid: Uuid },
    Cracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedAuth {
    PremiumName,
    OtherName,
}

struct CacheEntry {
    status: PremiumStatus,
    cached_at: Instant,
}

struct FailedAuthEntry {
    kind: FailedAuth,
    failed_at: Instant,
}

pub struct PremiumCache {
    entries: DashMap<String, CacheEntry>,
    ttl: Duration,
    failed_auths: DashMap<String, FailedAuthEntry>,
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
        let entry = self.entries.get(&username.to_lowercase())?;
        (entry.cached_at.elapsed() < self.ttl).then(|| entry.status.clone())
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

    pub fn mark_auth_failed(&self, username: &str) -> FailedAuth {
        let key = username.to_lowercase();
        let last_known = self.entries.get(&key).map(|entry| entry.status.clone());
        let kind = match last_known {
            Some(PremiumStatus::Premium { .. }) => FailedAuth::PremiumName,
            _ => FailedAuth::OtherName,
        };
        self.failed_auths.insert(
            key,
            FailedAuthEntry {
                kind,
                failed_at: Instant::now(),
            },
        );
        kind
    }

    pub fn failed_auth(&self, username: &str) -> Option<FailedAuth> {
        let key = username.to_lowercase();
        let entry = self.failed_auths.get(&key)?;
        if entry.failed_at.elapsed() < self.failed_auth_ttl {
            Some(entry.kind)
        } else {
            drop(entry);
            self.failed_auths.remove(&key);
            None
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

        assert_eq!(cache.failed_auth("Hypixel"), None);

        cache.mark_auth_failed("Hypixel");
        assert_eq!(cache.failed_auth("hypixel"), Some(FailedAuth::OtherName));
    }

    #[test]
    fn a_failed_auth_on_a_premium_name_is_remembered_as_such() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));
        cache.put(
            "Hypixel",
            PremiumStatus::Premium {
                mojang_uuid: Uuid::nil(),
            },
        );
        cache.put("Steve", PremiumStatus::Cracked);

        assert_eq!(cache.mark_auth_failed("Hypixel"), FailedAuth::PremiumName);
        assert_eq!(cache.mark_auth_failed("Steve"), FailedAuth::OtherName);

        cache.invalidate("Steve");
        assert_eq!(cache.failed_auth("hypixel"), Some(FailedAuth::PremiumName));
    }

    #[test]
    fn a_premium_name_stays_remembered_after_its_status_expires() {
        let cache = PremiumCache::new(Duration::from_millis(500), Duration::from_secs(60));
        cache.put(
            "Hypixel",
            PremiumStatus::Premium {
                mojang_uuid: Uuid::nil(),
            },
        );
        cache.mark_auth_failed("Hypixel");

        std::thread::sleep(Duration::from_millis(600));

        assert!(cache.get("Hypixel").is_none());
        assert_eq!(cache.failed_auth("Hypixel"), Some(FailedAuth::PremiumName));
    }

    #[test]
    fn a_failed_auth_counts_a_premium_name_whose_status_just_expired() {
        let cache = PremiumCache::new(Duration::ZERO, Duration::from_secs(60));
        cache.put(
            "Hypixel",
            PremiumStatus::Premium {
                mojang_uuid: Uuid::nil(),
            },
        );
        assert!(cache.get("Hypixel").is_none());

        assert_eq!(cache.mark_auth_failed("Hypixel"), FailedAuth::PremiumName);
    }

    #[test]
    fn failed_auth_expires() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_millis(0));

        cache.mark_auth_failed("Hypixel");
        assert_eq!(cache.failed_auth("Hypixel"), None);
    }

    #[test]
    fn invalidate_clears_failed_auth() {
        let cache = PremiumCache::new(Duration::from_secs(60), Duration::from_secs(60));

        cache.mark_auth_failed("Steve");
        assert!(cache.failed_auth("Steve").is_some());

        cache.invalidate("Steve");
        assert_eq!(cache.failed_auth("Steve"), None);
    }
}
