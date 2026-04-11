//! `DomainRouter` — thread-safe, incrementally updatable domain router.
//!
//! Replaces the immutable `DomainIndex` + `ArcSwap` pattern with
//! `DashMap` for lock-free concurrent reads and `RwLock` for the
//! (rarely-written) wildcard pattern list.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use wildmatch::WildMatch;

use infrarust_config::ServerConfig;

use crate::provider::ProviderId;

/// Entry stored per provider in the router.
struct RouterEntry {
    config: Arc<ServerConfig>,
    /// Normalized domains registered by this config (for cleanup on remove).
    domains: Vec<String>,
}

/// Pre-compiled wildcard pattern entry.
struct WildcardEntry {
    /// The original pattern string (for debug / rebuild).
    _raw: String,
    /// Compiled matcher.
    matcher: WildMatch,
    /// The provider that owns this pattern.
    provider_id: ProviderId,
}

use crate::util::normalize_handshake;

/// Returns `true` if the domain string contains wildcard characters.
fn is_wildcard(domain: &str) -> bool {
    domain.contains('*') || domain.contains('?')
}

/// Thread-safe domain router with incremental add/update/remove.
///
/// Uses `DashMap` for O(1) lock-free exact domain lookups and
/// `RwLock<Vec>` for sequential wildcard matching (rarely more
/// than a few dozen patterns in practice).
///
/// Exact matches always take priority over wildcard matches.
pub struct DomainRouter {
    /// `ProviderId` → config + registered domains.
    configs: DashMap<ProviderId, RouterEntry>,
    /// Exact domain (normalized lowercase) → `ProviderId`.
    exact_domains: DashMap<String, ProviderId>,
    /// Wildcard patterns, rebuilt when wildcard configs change.
    wildcard_patterns: RwLock<Vec<WildcardEntry>>,
}

impl DomainRouter {
    pub fn new() -> Self {
        Self {
            configs: DashMap::new(),
            exact_domains: DashMap::new(),
            wildcard_patterns: RwLock::new(Vec::new()),
        }
    }

    /// Adds a server configuration to the router.
    ///
    /// Registers all domains from the config. If a domain is already
    /// registered by another provider, the new registration wins
    /// (last-write-wins) and a warning is logged.
    pub fn add(&self, id: ProviderId, config: ServerConfig) {
        let mut registered_domains = Vec::new();
        let mut has_wildcards = false;

        for domain in &config.domains {
            let normalized = domain.to_lowercase();
            if is_wildcard(&normalized) {
                has_wildcards = true;
            } else {
                // Exact domain — last-write-wins
                if let Some(old) = self.exact_domains.insert(normalized.clone(), id.clone())
                    && old != id
                {
                    tracing::warn!(
                        domain = %normalized,
                        old_provider = %old,
                        new_provider = %id,
                        "domain conflict: overwriting previous provider"
                    );
                }
            }
            registered_domains.push(normalized);
        }

        self.configs.insert(
            id,
            RouterEntry {
                config: Arc::new(config),
                domains: registered_domains,
            },
        );

        if has_wildcards {
            self.rebuild_wildcards();
        }
    }

    /// Updates an existing configuration.
    ///
    /// Removes old domain entries and re-registers with the new config.
    pub fn update(&self, id: ProviderId, config: ServerConfig) {
        self.remove(&id);
        self.add(id, config);
    }

    /// Removes a configuration and all its domain entries.
    pub fn remove(&self, id: &ProviderId) {
        let removed = self.configs.remove(id);
        let Some((_, entry)) = removed else {
            return;
        };

        let mut had_wildcards = false;

        for domain in &entry.domains {
            if is_wildcard(domain) {
                had_wildcards = true;
            } else {
                // Only remove if this provider still owns the domain.
                self.exact_domains.remove_if(domain, |_, owner| owner == id);
            }
        }

        if had_wildcards {
            self.rebuild_wildcards();
        }
    }

    /// Removes all configurations from a given provider type.
    pub fn remove_all_by_provider_type(&self, provider_type: &str) {
        let ids: Vec<ProviderId> = self
            .configs
            .iter()
            .filter(|entry| entry.key().provider_type == provider_type)
            .map(|entry| entry.key().clone())
            .collect();

        for id in &ids {
            self.remove(id);
        }
    }

    /// Resolves a domain to its provider and server configuration.
    ///
    /// Exact matches take priority over wildcard matches.
    /// FML markers and trailing dots are stripped before resolution.
    pub fn resolve(&self, domain: &str) -> Option<(ProviderId, Arc<ServerConfig>)> {
        let stripped = normalize_handshake(domain);
        let normalized = stripped.to_lowercase();

        // 1. Exact match (O(1) via DashMap)
        if let Some(provider_id) = self.exact_domains.get(&normalized)
            && let Some(entry) = self.configs.get(provider_id.value())
        {
            return Some((provider_id.value().clone(), Arc::clone(&entry.config)));
        }

        // 2. Wildcard match (sequential scan)
        {
            let patterns = self
                .wildcard_patterns
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for wc in patterns.iter() {
                if wc.matcher.matches(&normalized)
                    && let Some(entry) = self.configs.get(&wc.provider_id)
                {
                    return Some((wc.provider_id.clone(), Arc::clone(&entry.config)));
                }
            }
        }

        None
    }

    /// Convenience: resolves a domain and returns only the config.
    pub fn resolve_config(&self, domain: &str) -> Option<Arc<ServerConfig>> {
        self.resolve(domain).map(|(_, cfg)| cfg)
    }

    pub fn find_by_server_id(&self, server_id: &str) -> Option<Arc<ServerConfig>> {
        self.configs
            .iter()
            .find(|entry| entry.value().config.effective_id() == server_id)
            .map(|entry| Arc::clone(&entry.value().config))
    }

    pub fn list_all(&self) -> Vec<(ProviderId, Arc<ServerConfig>)> {
        self.configs
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(&entry.value().config)))
            .collect()
    }

    pub fn count_by_provider(&self) -> HashMap<String, usize> {
        let mut counts = HashMap::new();
        for entry in &self.configs {
            *counts.entry(entry.key().provider_type.clone()).or_insert(0) += 1;
        }
        counts
    }

    pub fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }

    /// Rebuilds the wildcard pattern list from all configs.
    fn rebuild_wildcards(&self) {
        let mut patterns = Vec::new();

        for entry in &self.configs {
            for domain in &entry.value().domains {
                if is_wildcard(domain) {
                    patterns.push(WildcardEntry {
                        _raw: domain.clone(),
                        matcher: WildMatch::new(domain),
                        provider_id: entry.key().clone(),
                    });
                }
            }
        }

        let mut lock = self
            .wildcard_patterns
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *lock = patterns;
    }
}

impl Default for DomainRouter {
    fn default() -> Self {
        Self::new()
    }
}
