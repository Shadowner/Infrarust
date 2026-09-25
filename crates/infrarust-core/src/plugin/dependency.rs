//! Topological sort for plugin dependency resolution (Kahn's algorithm).

use std::collections::{BTreeSet, HashMap, HashSet};

use infrarust_api::error::PluginError;
use infrarust_api::plugin::PluginMetadata;

/// Resolves the load order of plugins via topological sort.
///
/// Returns plugin IDs in the order they should be loaded.
///
/// # Errors
/// - Missing non-optional dependency
/// - Circular dependency detected
pub fn resolve_load_order(plugins: &[PluginMetadata]) -> Result<Vec<String>, PluginError> {
    let available: HashSet<&str> = plugins.iter().map(|p| p.id.as_str()).collect();

    // 1. Check that all required dependencies are present
    for plugin in plugins {
        for dep in &plugin.dependencies {
            if !dep.optional && !available.contains(dep.id.as_str()) {
                return Err(PluginError::InitFailed(format!(
                    "Plugin '{}' requires '{}' which is not loaded",
                    plugin.id, dep.id
                )));
            }
        }
    }

    // 2. Build the dependency graph
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for plugin in plugins {
        in_degree.entry(plugin.id.as_str()).or_insert(0);
        for dep in &plugin.dependencies {
            if available.contains(dep.id.as_str()) {
                *in_degree.entry(plugin.id.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.id.as_str())
                    .or_default()
                    .push(plugin.id.as_str());
            }
        }
    }

    // 3. Kahn's algorithm
    let position: HashMap<&str, usize> = plugins
        .iter()
        .enumerate()
        .map(|(at, plugin)| (plugin.id.as_str(), at))
        .collect();
    let mut ready: BTreeSet<usize> = plugins
        .iter()
        .enumerate()
        .filter(|(_, plugin)| in_degree.get(plugin.id.as_str()) == Some(&0))
        .map(|(at, _)| at)
        .collect();

    let mut sorted: Vec<String> = Vec::with_capacity(plugins.len());

    while let Some(at) = ready.pop_first() {
        let node = plugins[at].id.as_str();
        sorted.push(node.to_string());
        if let Some(deps) = dependents.get(node) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0
                        && let Some(&dep_at) = position.get(dep)
                    {
                        ready.insert(dep_at);
                    }
                }
            }
        }
    }

    // 4. Cycle detection
    if sorted.len() != plugins.len() {
        let remaining: Vec<&str> = in_degree
            .iter()
            .filter(|(_, deg)| **deg > 0)
            .map(|(id, _)| *id)
            .collect();
        return Err(PluginError::InitFailed(format!(
            "Circular dependency detected involving: {}",
            remaining.join(", ")
        )));
    }

    Ok(sorted)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn ids(order: &[String]) -> Vec<&str> {
        order.iter().map(String::as_str).collect()
    }

    #[test]
    fn independent_plugins_keep_their_discovery_order() {
        let plugins: Vec<PluginMetadata> = (0..32)
            .map(|i| PluginMetadata::new(format!("p{i:02}"), "p", "1.0.0"))
            .collect();
        let expected: Vec<String> = plugins.iter().map(|p| p.id.clone()).collect();
        assert_eq!(resolve_load_order(&plugins).unwrap(), expected);
    }

    #[test]
    fn a_dependency_only_moves_its_dependent() {
        let plugins = vec![
            PluginMetadata::new("lobby", "l", "1.0.0").optional_dependency("auth"),
            PluginMetadata::new("stats", "s", "1.0.0"),
            PluginMetadata::new("auth", "a", "1.0.0"),
            PluginMetadata::new("hub", "h", "1.0.0").depends_on("stats"),
        ];
        let order = resolve_load_order(&plugins).unwrap();
        assert_eq!(ids(&order), ["stats", "auth", "lobby", "hub"]);
    }
}
