//! Pre-compiled domain index for fast hostname resolution.

use std::collections::HashMap;

use wildmatch::WildMatch;

use crate::server::ServerConfig;

/// Index pré-compilé pour la résolution de domaines.
///
/// Résout le problème V1 du O(n×m) avec recompilation du `WildMatch`
/// à chaque requête. Ici les patterns sont compilés une seule fois
/// au chargement (et recompilés au hot-reload).
///
/// Stratégie :
/// - Les domaines exacts vont dans un `HashMap` → O(1)
/// - Les patterns wildcard sont pré-compilés et testés séquentiellement
///   (rarement plus de 10-20 patterns en pratique)
/// - Les domaines exacts sont prioritaires sur les wildcards
pub struct DomainIndex {
    /// Domaines exacts → `config_id`. Lookup O(1).
    exact: HashMap<String, String>,
    /// Patterns wildcard pré-compilés, testés dans l'ordre d'insertion.
    wildcards: Vec<CompiledPattern>,
}

struct CompiledPattern {
    /// Le pattern original (pour debug/affichage).
    raw: String,
    /// Le pattern compilé.
    matcher: WildMatch,
    /// L'identifiant de la config associée.
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
    /// Construit l'index à partir d'une liste de configs.
    ///
    /// Les domaines sont normalisés en lowercase.
    /// Les domaines sans wildcard vont dans la `HashMap` exacte.
    /// Les domaines avec `*` ou `?` vont dans la liste wildcard.
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

    /// Résout un domaine vers l'identifiant de config.
    ///
    /// Les domaines exacts sont prioritaires sur les wildcards.
    /// FML markers are stripped before resolution.
    /// Retourne `None` si aucun pattern ne matche.
    pub fn resolve(&self, domain: &str) -> Option<&str> {
        let stripped = strip_fml_marker(domain);
        let normalized = stripped.to_lowercase();

        // 1. Exact match (O(1))
        if let Some(id) = self.exact.get(&normalized) {
            return Some(id.as_str());
        }

        // 2. Wildcard match (séquentiel, patterns pré-compilés)
        for pattern in &self.wildcards {
            if pattern.matcher.matches(&normalized) {
                return Some(pattern.config_id.as_str());
            }
        }

        None
    }

    /// Nombre total de patterns indexés.
    pub fn len(&self) -> usize {
        self.exact.len() + self.wildcards.len()
    }

    /// `true` si l'index est vide.
    pub fn is_empty(&self) -> bool {
        self.exact.is_empty() && self.wildcards.is_empty()
    }
}
