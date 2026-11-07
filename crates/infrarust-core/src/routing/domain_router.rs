//! `DomainRouter` — thread-safe, incrementally updatable domain router.
//!
//! Replaces the immutable `DomainIndex` + `ArcSwap` pattern with
//! `DashMap` for lock-free concurrent reads and `RwLock` for the
//! (rarely-written) wildcard pattern list.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use dashmap::DashMap;
use wildmatch::WildMatch;

use infrarust_config::ServerConfig;

use crate::loadbalancer::{LoadBalancer, build_load_balancer};
use crate::provider::{ProviderId, ProviderTrust};
use crate::util::sync::{lock, read, write};

/// Entry stored per provider in the router.
#[derive(Clone)]
struct RouterEntry {
    config: Arc<ServerConfig>,
    /// Normalized domains registered by this config.
    domains: Vec<String>,
    /// One LB instance per config (RR/SWRR state is per-server). Rebuilt
    /// on add/update, so a config change resets the rotation state.
    load_balancer: Arc<dyn LoadBalancer>,
    /// Registration order: the latest add or update has the highest value.
    seq: u64,
}

impl RouterEntry {
    /// The server id this config asks for. A config with neither `name` nor
    /// `id` asks for none, even though it resolves to `"unknown"`.
    fn claimed_server_id(&self) -> Option<&str> {
        self.config.name.as_deref().or(self.config.id.as_deref())
    }
}

/// Every config providers currently submit, routed or held back.
#[derive(Default)]
struct Registrations {
    next_seq: u64,
    entries: HashMap<ProviderId, RouterEntry>,
}

impl Registrations {
    fn insert(
        &mut self,
        id: ProviderId,
        config: ServerConfig,
        load_balancer: Arc<dyn LoadBalancer>,
    ) {
        let domains = config.domains.iter().map(|d| d.to_lowercase()).collect();
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.insert(
            id,
            RouterEntry {
                config: Arc::new(config),
                domains,
                load_balancer,
                seq,
            },
        );
    }

    /// The configs to route, best first. A config is held back while a more
    /// trusted one claims the same server id.
    fn routed(&self) -> Vec<(&ProviderId, &RouterEntry)> {
        let mut strongest: HashMap<&str, ProviderTrust> = HashMap::new();
        for (id, entry) in &self.entries {
            if let Some(server_id) = entry.claimed_server_id() {
                let trust = strongest.entry(server_id).or_insert_with(|| id.trust());
                *trust = (*trust).max(id.trust());
            }
        }

        let mut routed: Vec<_> = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                entry
                    .claimed_server_id()
                    .is_none_or(|server_id| strongest.get(server_id) == Some(&id.trust()))
            })
            .collect();
        routed.sort_unstable_by_key(|(id, entry)| Reverse((id.trust(), entry.seq)));
        routed
    }
}

/// Configs that entered or left routing during a router mutation.
#[derive(Default)]
pub struct RoutingChanges {
    pub entered: Vec<ProviderId>,
    pub left: Vec<(ProviderId, Arc<ServerConfig>)>,
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
/// Exact matches always take priority over wildcard matches. Between configs
/// claiming the same domain, the more trusted provider wins, then the latest
/// registration. Removing the winner hands the domain to the next claimant.
pub struct DomainRouter {
    /// Source of truth. Everything below is derived from it by `publish`.
    registrations: Mutex<Registrations>,
    /// `ProviderId` → routed config.
    configs: DashMap<ProviderId, RouterEntry>,
    /// Exact domain (normalized lowercase) → `ProviderId`.
    exact_domains: DashMap<String, ProviderId>,
    /// Wildcard patterns, best owner first.
    wildcard_patterns: RwLock<Vec<WildcardEntry>>,
}

impl DomainRouter {
    pub fn new() -> Self {
        Self {
            registrations: Mutex::new(Registrations::default()),
            configs: DashMap::new(),
            exact_domains: DashMap::new(),
            wildcard_patterns: RwLock::new(Vec::new()),
        }
    }

    /// Adds a server configuration to the router.
    pub fn add(&self, id: ProviderId, config: ServerConfig) -> RoutingChanges {
        let mut registrations = lock(&self.registrations);
        let load_balancer = build_load_balancer(&config.balance_config());
        registrations.insert(id.clone(), config, load_balancer);
        self.publish(&registrations, Some(&id))
    }

    /// Updates an existing configuration.
    ///
    /// The load balancer instance survives when nothing it depends on
    /// changed, so an unrelated edit does not reset the rotation state.
    pub fn update(&self, id: ProviderId, config: ServerConfig) -> RoutingChanges {
        let mut registrations = lock(&self.registrations);
        let load_balancer = registrations
            .entries
            .get(&id)
            .filter(|entry| {
                entry.config.balance_config() == config.balance_config()
                    && entry.config.addresses == config.addresses
            })
            .map_or_else(
                || build_load_balancer(&config.balance_config()),
                |entry| Arc::clone(&entry.load_balancer),
            );
        registrations.insert(id.clone(), config, load_balancer);
        self.publish(&registrations, Some(&id))
    }

    /// Removes a configuration and all its domain entries.
    pub fn remove(&self, id: &ProviderId) -> RoutingChanges {
        let mut registrations = lock(&self.registrations);
        if registrations.entries.remove(id).is_none() {
            return RoutingChanges::default();
        }
        self.publish(&registrations, None)
    }

    /// Removes all configurations from a given provider type.
    pub fn remove_all_by_provider_type(&self, provider_type: &str) -> RoutingChanges {
        let mut registrations = lock(&self.registrations);
        registrations
            .entries
            .retain(|id, _| id.provider_type != provider_type);
        self.publish(&registrations, None)
    }

    /// Brings the routing views in line with `registrations`. Additions land
    /// before retirements, so a domain that stays routed never misses.
    fn publish(
        &self,
        registrations: &Registrations,
        submitted: Option<&ProviderId>,
    ) -> RoutingChanges {
        let routed = registrations.routed();
        let routed_ids: HashSet<&ProviderId> = routed.iter().map(|(id, _)| *id).collect();
        let mut changes = RoutingChanges::default();

        for (id, entry) in &routed {
            let current_seq = self.configs.get(*id).map(|current| current.seq);
            if current_seq != Some(entry.seq) {
                self.configs.insert((*id).clone(), (*entry).clone());
            }
            if current_seq.is_none() {
                changes.entered.push((*id).clone());
            }
        }

        let mut exact: HashMap<&str, &ProviderId> = HashMap::new();
        let mut wildcards = Vec::new();
        for (id, entry) in &routed {
            for domain in &entry.domains {
                if is_wildcard(domain) {
                    wildcards.push(WildcardEntry {
                        _raw: domain.clone(),
                        matcher: WildMatch::new(domain),
                        provider_id: (*id).clone(),
                    });
                } else {
                    exact.entry(domain).or_insert(*id);
                }
            }
        }

        for (domain, owner) in &exact {
            if self
                .exact_domains
                .get(*domain)
                .is_some_and(|current| current.value() == *owner)
            {
                continue;
            }
            if let Some(previous) = self
                .exact_domains
                .insert((*domain).to_string(), (*owner).clone())
                && routed_ids.contains(&previous)
            {
                tracing::warn!(
                    domain = %domain,
                    old_provider = %previous,
                    new_provider = %owner,
                    "domain conflict: overwriting previous provider"
                );
            }
        }
        self.exact_domains
            .retain(|domain, _| exact.contains_key(domain.as_str()));

        *write(&self.wildcard_patterns) = wildcards;

        self.configs.retain(|id, entry| {
            let keep = routed_ids.contains(id);
            if !keep {
                changes.left.push((id.clone(), Arc::clone(&entry.config)));
            }
            keep
        });

        if let Some(id) = submitted
            && routed_ids.contains(id)
            && let Some(entry) = registrations.entries.get(id)
        {
            for domain in &entry.domains {
                if let Some(owner) = exact.get(domain.as_str())
                    && owner.trust() > id.trust()
                {
                    tracing::warn!(
                        domain = %domain,
                        provider = %id,
                        owner = %owner,
                        "domain conflict: a more trusted provider keeps this domain"
                    );
                }
            }
        }

        let displaced = changes.left.iter().map(|(id, _)| id);
        for id in submitted.into_iter().chain(displaced) {
            if !routed_ids.contains(id)
                && let Some(entry) = registrations.entries.get(id)
            {
                tracing::warn!(
                    provider = %id,
                    server = entry.claimed_server_id().unwrap_or_default(),
                    "a more trusted provider claims this server id, holding this config back until it goes away"
                );
            }
        }

        changes
    }

    /// Resolves a domain to its provider and server configuration.
    ///
    /// Exact matches take priority over wildcard matches.
    /// FML markers and trailing dots are stripped before resolution.
    pub fn resolve(&self, domain: &str) -> Option<(ProviderId, Arc<ServerConfig>)> {
        self.resolve_route(domain).map(|(id, cfg, _lb)| (id, cfg))
    }

    pub fn resolve_route(
        &self,
        domain: &str,
    ) -> Option<(ProviderId, Arc<ServerConfig>, Arc<dyn LoadBalancer>)> {
        let stripped = normalize_handshake(domain);
        let normalized = stripped.to_lowercase();

        // 1. Exact match (O(1) via DashMap)
        if let Some(provider_id) = self.exact_domains.get(&normalized)
            && let Some(entry) = self.configs.get(provider_id.value())
        {
            return Some((
                provider_id.value().clone(),
                Arc::clone(&entry.config),
                Arc::clone(&entry.load_balancer),
            ));
        }

        // 2. Wildcard match (sequential scan)
        {
            let patterns = read(&self.wildcard_patterns);
            for wc in patterns.iter() {
                if wc.matcher.matches(&normalized)
                    && let Some(entry) = self.configs.get(&wc.provider_id)
                {
                    return Some((
                        wc.provider_id.clone(),
                        Arc::clone(&entry.config),
                        Arc::clone(&entry.load_balancer),
                    ));
                }
            }
        }

        None
    }

    pub fn get(&self, id: &ProviderId) -> Option<Arc<ServerConfig>> {
        self.configs
            .get(id)
            .map(|entry| Arc::clone(&entry.value().config))
    }

    pub fn find_by_server_id(&self, server_id: &str) -> Option<Arc<ServerConfig>> {
        self.find_route_by_server_id(server_id)
            .map(|(cfg, _lb)| cfg)
    }

    pub fn find_route_by_server_id(
        &self,
        server_id: &str,
    ) -> Option<(Arc<ServerConfig>, Arc<dyn LoadBalancer>)> {
        self.configs
            .iter()
            .filter(|entry| entry.value().config.effective_id() == server_id)
            .map(|entry| {
                (
                    entry.value().seq,
                    Arc::clone(&entry.value().config),
                    Arc::clone(&entry.value().load_balancer),
                )
            })
            .max_by_key(|(seq, _, _)| *seq)
            .map(|(_, config, load_balancer)| (config, load_balancer))
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
}

impl Default for DomainRouter {
    fn default() -> Self {
        Self::new()
    }
}
