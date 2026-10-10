//! Topological sort for plugin dependency resolution (Kahn's algorithm).

use std::collections::{BTreeSet, HashMap, HashSet};

use infrarust_api::plugin::PluginMetadata;

use super::error::PluginManagerError;

pub struct LoadOrder {
    pub order: Vec<String>,
    pub refused: Vec<PluginManagerError>,
}

pub fn resolve_load_order(plugins: &[PluginMetadata]) -> LoadOrder {
    let mut position: HashMap<&str, usize> = HashMap::new();
    for (at, plugin) in plugins.iter().enumerate() {
        position.entry(plugin.id.as_str()).or_insert(at);
    }
    let mut refusals: Vec<Option<PluginManagerError>> = plugins.iter().map(|_| None).collect();

    for (at, plugin) in plugins.iter().enumerate() {
        if let Some(missing) = plugin
            .dependencies
            .iter()
            .find(|dep| !dep.optional && !position.contains_key(dep.id.as_str()))
        {
            refusals[at] = Some(PluginManagerError::MissingDependency {
                plugin: plugin.id.clone(),
                dependency: missing.id.clone(),
            });
        }
    }

    loop {
        refuse_dependents(plugins, &position, &mut refusals);
        let (order, stuck) = topological_order(plugins, &position, &refusals);
        if stuck.is_empty() {
            return LoadOrder {
                order,
                refused: refusals.into_iter().flatten().collect(),
            };
        }
        refuse_cycles(plugins, &position, &stuck, &mut refusals);
    }
}

fn refuse_dependents(
    plugins: &[PluginMetadata],
    position: &HashMap<&str, usize>,
    refusals: &mut [Option<PluginManagerError>],
) {
    loop {
        let mut changed = false;
        for (at, plugin) in plugins.iter().enumerate() {
            if refusals[at].is_some() {
                continue;
            }
            let refused_dependency = plugin.dependencies.iter().find(|dep| {
                !dep.optional
                    && position
                        .get(dep.id.as_str())
                        .is_some_and(|&dep_at| refusals[dep_at].is_some())
            });
            if let Some(dep) = refused_dependency {
                refusals[at] = Some(PluginManagerError::DependencyNotEnabled {
                    plugin: plugin.id.clone(),
                    dependency: dep.id.clone(),
                });
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

fn topological_order(
    plugins: &[PluginMetadata],
    position: &HashMap<&str, usize>,
    refusals: &[Option<PluginManagerError>],
) -> (Vec<String>, Vec<usize>) {
    let live = |at: usize| refusals[at].is_none();
    let mut in_degree = vec![0usize; plugins.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); plugins.len()];
    for (at, plugin) in plugins.iter().enumerate() {
        if !live(at) {
            continue;
        }
        for dep in &plugin.dependencies {
            if let Some(&dep_at) = position.get(dep.id.as_str())
                && live(dep_at)
            {
                in_degree[at] += 1;
                dependents[dep_at].push(at);
            }
        }
    }

    let mut ready: BTreeSet<usize> = (0..plugins.len())
        .filter(|&at| live(at) && in_degree[at] == 0)
        .collect();
    let mut order = Vec::with_capacity(plugins.len());
    while let Some(at) = ready.pop_first() {
        order.push(plugins[at].id.clone());
        for &dependent in &dependents[at] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                ready.insert(dependent);
            }
        }
    }

    let stuck = (0..plugins.len())
        .filter(|&at| live(at) && in_degree[at] > 0)
        .collect();
    (order, stuck)
}

fn refuse_cycles(
    plugins: &[PluginMetadata],
    position: &HashMap<&str, usize>,
    stuck: &[usize],
    refusals: &mut [Option<PluginManagerError>],
) {
    let in_stuck: HashSet<usize> = stuck.iter().copied().collect();
    let edges = |at: usize| -> Vec<usize> {
        plugins[at]
            .dependencies
            .iter()
            .filter_map(|dep| position.get(dep.id.as_str()).copied())
            .filter(|dep_at| in_stuck.contains(dep_at))
            .collect()
    };
    let reach: HashMap<usize, HashSet<usize>> = stuck
        .iter()
        .map(|&at| {
            let mut seen = HashSet::new();
            let mut pending = edges(at);
            while let Some(next) = pending.pop() {
                if seen.insert(next) {
                    pending.extend(edges(next));
                }
            }
            (at, seen)
        })
        .collect();

    for &at in stuck {
        if !reach[&at].contains(&at) {
            continue;
        }
        let cycle = stuck
            .iter()
            .filter(|&&other| {
                other == at || (reach[&at].contains(&other) && reach[&other].contains(&at))
            })
            .map(|&other| plugins[other].id.clone())
            .collect();
        refusals[at] = Some(PluginManagerError::Cycle {
            plugin: plugins[at].id.clone(),
            cycle,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn resolved(plugins: &[PluginMetadata]) -> (Vec<String>, Vec<String>) {
        let resolution = resolve_load_order(plugins);
        let refused = resolution.refused.iter().map(ToString::to_string).collect();
        (resolution.order, refused)
    }

    #[test]
    fn independent_plugins_keep_their_discovery_order() {
        let plugins: Vec<PluginMetadata> = (0..32)
            .map(|i| PluginMetadata::new(format!("p{i:02}"), "p", "1.0.0"))
            .collect();
        let expected: Vec<String> = plugins.iter().map(|p| p.id.clone()).collect();
        assert_eq!(resolved(&plugins), (expected, Vec::new()));
    }

    #[test]
    fn a_dependency_only_moves_its_dependent() {
        let plugins = vec![
            PluginMetadata::new("lobby", "l", "1.0.0").optional_dependency("auth"),
            PluginMetadata::new("stats", "s", "1.0.0"),
            PluginMetadata::new("auth", "a", "1.0.0"),
            PluginMetadata::new("hub", "h", "1.0.0").depends_on("stats"),
        ];
        let (order, refused) = resolved(&plugins);
        assert_eq!(order, ["stats", "auth", "lobby", "hub"]);
        assert!(refused.is_empty(), "{refused:?}");
    }

    #[test]
    fn a_missing_dependency_refuses_its_plugin_and_every_hard_dependent() {
        let plugins = vec![
            PluginMetadata::new("needy", "n", "1.0.0").depends_on("absent"),
            PluginMetadata::new("rider", "r", "1.0.0").depends_on("needy"),
            PluginMetadata::new("top", "t", "1.0.0").depends_on("rider"),
            PluginMetadata::new("relaxed", "x", "1.0.0").optional_dependency("needy"),
            PluginMetadata::new("good", "g", "1.0.0"),
        ];
        let (order, refused) = resolved(&plugins);
        assert_eq!(order, ["relaxed", "good"]);
        assert_eq!(
            refused,
            [
                "plugin 'needy' requires 'absent', which was not found",
                "plugin 'rider' requires 'needy', which is not enabled",
                "plugin 'top' requires 'rider', which is not enabled",
            ]
        );
    }

    #[test]
    fn a_cycle_refuses_its_members_and_their_hard_dependents_only() {
        let plugins = vec![
            PluginMetadata::new("ping", "p", "1.0.0").depends_on("pong"),
            PluginMetadata::new("pong", "p", "1.0.0").depends_on("ping"),
            PluginMetadata::new("fan", "f", "1.0.0").depends_on("ping"),
            PluginMetadata::new("watcher", "w", "1.0.0").optional_dependency("pong"),
            PluginMetadata::new("good", "g", "1.0.0"),
        ];
        let (order, refused) = resolved(&plugins);
        assert_eq!(order, ["watcher", "good"]);
        assert_eq!(
            refused,
            [
                "plugin 'ping' is refused: its dependencies form a cycle (ping, pong)",
                "plugin 'pong' is refused: its dependencies form a cycle (ping, pong)",
                "plugin 'fan' requires 'ping', which is not enabled",
            ]
        );
    }

    #[test]
    fn a_plugin_depending_on_itself_is_a_cycle_of_one() {
        let plugins = vec![
            PluginMetadata::new("selfish", "s", "1.0.0").depends_on("selfish"),
            PluginMetadata::new("good", "g", "1.0.0"),
        ];
        let (order, refused) = resolved(&plugins);
        assert_eq!(order, ["good"]);
        assert_eq!(
            refused,
            ["plugin 'selfish' is refused: its dependencies form a cycle (selfish)"]
        );
    }

    #[test]
    fn two_separate_cycles_are_each_named_on_their_own() {
        let plugins = vec![
            PluginMetadata::new("a", "a", "1.0.0").depends_on("b"),
            PluginMetadata::new("b", "b", "1.0.0").depends_on("a"),
            PluginMetadata::new("c", "c", "1.0.0").depends_on("d"),
            PluginMetadata::new("d", "d", "1.0.0").optional_dependency("c"),
        ];
        let (order, refused) = resolved(&plugins);
        assert!(order.is_empty(), "{order:?}");
        assert_eq!(
            refused,
            [
                "plugin 'a' is refused: its dependencies form a cycle (a, b)",
                "plugin 'b' is refused: its dependencies form a cycle (a, b)",
                "plugin 'c' is refused: its dependencies form a cycle (c, d)",
                "plugin 'd' is refused: its dependencies form a cycle (c, d)",
            ]
        );
    }
}
