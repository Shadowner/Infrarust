//! Topological sort for plugin dependency resolution (Kahn's algorithm).

use std::collections::{HashMap, HashSet, VecDeque};

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
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| *id)
        .collect();

    let mut sorted: Vec<String> = Vec::with_capacity(plugins.len());

    while let Some(node) = queue.pop_front() {
        sorted.push(node.to_string());
        if let Some(deps) = dependents.get(node) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep);
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
