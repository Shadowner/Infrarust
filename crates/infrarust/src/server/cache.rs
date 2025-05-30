use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

use infrarust_config::{InfrarustConfig, LogType, models::server::MotdConfig};
use infrarust_protocol::version::Version;
use tracing::{debug, instrument};

use crate::{
    network::{packet::Packet, proxy_protocol::ProtocolResult},
    server::{ServerRequest, backend::Server, motd},
};

#[derive(Debug)]
pub struct StatusCache {
    ttl: Duration,
    entries: HashMap<u64, CacheEntry>,
    max_size: usize,
    motd_config: MotdConfig,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    expires_at: SystemTime,
    response: Packet,
}

impl StatusCache {
    pub fn new(ttl: Duration, max_size: usize, motd_config: MotdConfig) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
            max_size,
            motd_config,
        }
    }

    pub fn from_shared_config(config: &InfrarustConfig) -> Self {
        let ttl = Duration::from_secs(config.cache.status_ttl_seconds.unwrap_or(30));
        let max_size = config.cache.max_status_entries.unwrap_or(1000);
        let motd_config = config.motds.unreachable.clone().unwrap_or_default();

        Self::new(ttl, max_size, motd_config)
    }

    #[instrument(name = "get_status_response", skip(self, server), fields(
        server_addr = %server.config.addresses.first().unwrap_or(&String::new()),
        protocol_version = ?req.protocol_version
    ))]
    pub async fn get_status_response(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Packet> {
        let key = self.cache_key(server, req.protocol_version);
        debug!(
            log_type = LogType::Cache.as_str(),
            "Status lookup for domain: {}, cache key: {}", req.domain, key
        );

        if let Some(cached) = self.check_cache(key) {
            return Ok(cached);
        }

        debug!(
            log_type = LogType::Cache.as_str(),
            "Cache miss for {}, fetching from server", req.domain
        );
        match server.fetch_status_directly(req).await {
            Ok(response) => {
                debug!(
                    log_type = LogType::Cache.as_str(),
                    "Server fetch successful for {}", req.domain
                );
                self.update_cache(key, response.clone());
                Ok(response)
            }
            Err(e) => {
                debug!(
                    log_type = LogType::Cache.as_str(),
                    "Server fetch failed for {}: {}", req.domain, e
                );
                // Use the dedicated motd function
                motd::handle_server_fetch_error(&server.config, &req.domain, &self.motd_config)
                    .await
            }
        }
    }

    fn check_cache(&self, key: u64) -> Option<Packet> {
        if let Some(entry) = self.entries.get(&key) {
            if entry.expires_at > SystemTime::now() {
                debug!(
                    log_type = LogType::Cache.as_str(),
                    "Cache hit for key: {}", key
                );
                return Some(entry.response.clone());
            }
            debug!(
                log_type = LogType::Cache.as_str(),
                "Cache entry expired for key: {}", key
            );
        }
        None
    }

    fn update_cache(&mut self, key: u64, response: Packet) {
        // Clean expired entries before adding new ones
        self.clean_expired_entries();

        // If cache is at max size, remove oldest entries
        if self.entries.len() >= self.max_size {
            self.remove_oldest_entries(10); // Remove 10% or at least one entry
        }

        self.entries.insert(
            key,
            CacheEntry {
                expires_at: SystemTime::now() + self.ttl,
                response,
            },
        );

        debug!(
            log_type = LogType::Cache.as_str(),
            "Cache updated for key: {}, cache size: {}",
            key,
            self.entries.len()
        );
    }

    fn clean_expired_entries(&mut self) {
        let now = SystemTime::now();
        let expired_keys: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(key, _)| *key)
            .collect();

        if !expired_keys.is_empty() {
            for key in expired_keys.iter() {
                self.entries.remove(key);
            }
            debug!(
                log_type = LogType::Cache.as_str(),
                "Removed {} expired entries from cache",
                expired_keys.len()
            );
        }
    }

    fn remove_oldest_entries(&mut self, count: usize) {
        // Sort by expiration time to find oldest entries
        let mut entries_with_keys: Vec<(u64, SystemTime)> = self
            .entries
            .iter()
            .map(|(key, entry)| (*key, entry.expires_at))
            .collect();

        entries_with_keys.sort_by(|(_, time1), (_, time2)| time1.cmp(time2));

        // Calculate how many to remove (at least one)
        let remove_count = std::cmp::max(1, std::cmp::min(count, entries_with_keys.len()));

        // Remove the oldest entries
        for (key, _) in entries_with_keys.iter().take(remove_count) {
            self.entries.remove(key);
        }

        debug!(
            log_type = LogType::Cache.as_str(),
            "Removed {} oldest entries to stay within size limit", remove_count
        );
    }

    pub fn cache_key(&self, server: &Server, version: Version) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        if !server.config.addresses.is_empty() {
            server.config.addresses[0].hash(&mut hasher);
        }
        version.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn check_cache_only(
        &mut self,
        server: &Server,
        req: &ServerRequest,
    ) -> ProtocolResult<Option<Packet>> {
        let key = self.cache_key(server, req.protocol_version);
        debug!(
            log_type = LogType::Cache.as_str(),
            "Quick cache check for domain: {} (key: {})", req.domain, key
        );

        Ok(self.check_cache(key))
    }

    pub async fn update_cache_for(
        &mut self,
        server: &Server,
        req: &ServerRequest,
        response: Packet,
    ) {
        let key = self.cache_key(server, req.protocol_version);
        debug!(
            log_type = LogType::Cache.as_str(),
            "Updating cache for domain: {} (key: {})", req.domain, key
        );
        self.update_cache(key, response);
    }
}
