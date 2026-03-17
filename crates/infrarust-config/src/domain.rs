//! Pre-compiled domain index for fast hostname resolution.

use std::collections::HashMap;

use wildmatch::WildMatch;

use crate::server::ServerConfig;

/// Pre-compiled index for domain resolution.
///
/// Solves the V1 problem of O(n*m) with `WildMatch` recompilation
/// on every request. Here patterns are compiled once
/// at load time (and recompiled on hot-reload).
///
/// Strategy:
/// - Exact domains go into a `HashMap` for O(1) lookup
/// - Wildcard patterns are pre-compiled and tested sequentially
///   (rarely more than 10-20 patterns in practice)
/// - Exact domains take priority over wildcards
pub struct DomainIndex {
    /// Exact domains to `config_id`. O(1) lookup.
    exact: HashMap<String, String>,
    /// Pre-compiled wildcard patterns, tested in insertion order.
    wildcards: Vec<CompiledPattern>,
}

struct CompiledPattern {
    /// The original pattern (for debug/display).
    raw: String,
    /// The compiled pattern.
    matcher: WildMatch,
    /// The associated config identifier.
    config_id: String,
}

impl std::fmt::Display for CompiledPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

/// Strip FML markers (`\0FML`, `\0FML2`, `\0FML3`) appended by
/// Forge/Fabric clients in the handshake hostname.
fn strip_fml_marker(domain: &str) -> &str {
    domain.split('\0').next().unwrap_or(domain)
}

impl DomainIndex {
    /// Builds the index from a list of configs.
    ///
    /// Domains are normalized to lowercase.
    /// Domains without wildcards go into the exact `HashMap`.
    /// Domains with `*` or `?` go into the wildcard list.
    pub fn build(configs: &[ServerConfig]) -> Self {
        let mut exact = HashMap::new();
        let mut wildcards = Vec::new();

        for config in configs {
            let id = config.effective_id();
            for domain in &config.domains {
                let normalized = domain.to_lowercase();
                if normalized.contains('*') || normalized.contains('?') {
                    wildcards.push(CompiledPattern {
                        raw: normalized.clone(),
                        matcher: WildMatch::new(&normalized),
                        config_id: id.clone(),
                    });
                } else {
                    exact.insert(normalized, id.clone());
                }
            }
        }

        Self { exact, wildcards }
    }

    /// Resolves a domain to its config identifier.
    ///
    /// Exact domains take priority over wildcards.
    /// FML markers are stripped before resolution.
    /// Returns `None` if no pattern matches.
    pub fn resolve(&self, domain: &str) -> Option<&str> {
        let stripped = strip_fml_marker(domain);
        let normalized = stripped.to_lowercase();

        // 1. Exact match (O(1))
        if let Some(id) = self.exact.get(&normalized) {
            return Some(id.as_str());
        }

        // 2. Wildcard match (sequential, pre-compiled patterns)
        for pattern in &self.wildcards {
            if pattern.matcher.matches(&normalized) {
                return Some(pattern.config_id.as_str());
            }
        }

        None
    }

    /// Returns the total number of indexed patterns.
    pub fn len(&self) -> usize {
        self.exact.len() + self.wildcards.len()
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.wildcards.is_empty()
    }
}
