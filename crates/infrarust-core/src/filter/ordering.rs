//! Filter ordering via topological sort (Kahn's algorithm).
//!
//! Resolves the execution order of filters based on their `after`/`before`
//! dependencies and `FilterPriority` as a tiebreaker.

use std::collections::{BinaryHeap, HashMap, HashSet};
use std::cmp::Reverse;

use infrarust_api::filter::{FilterMetadata, FilterPriority};

/// Error during filter order resolution.
#[derive(Debug, thiserror::Error)]
pub enum FilterOrderError {
    /// A circular dependency was detected.
    #[error("circular dependency detected involving: {0}")]
    CyclicDependency(String),
}

/// Helper for priority-based tiebreaking in Kahn's algorithm.
#[derive(Eq, PartialEq)]
struct PriorityEntry<'a> {
    id: &'a str,
    priority: FilterPriority,
    insertion_order: usize,
}

impl Ord for PriorityEntry<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap: largest element is popped first.
        // We want First(0) before Last(4), and lower insertion_order first.
        // Reverse both so the "smallest" values become "largest" in the heap.
        Reverse(self.priority)
            .cmp(&Reverse(other.priority))
            .then_with(|| Reverse(self.insertion_order).cmp(&Reverse(other.insertion_order)))
    }
}

impl PartialOrd for PriorityEntry<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Resolves filter execution order from metadata.
///
/// Uses Kahn's algorithm with `after`/`before` dependencies.
/// `before` constraints are converted to `after` on the target.
/// When multiple filters have no dependency relationship,
/// `FilterPriority` is used as a tiebreaker (then insertion order).
pub fn resolve_filter_order(
    filters: &[FilterMetadata],
) -> Result<Vec<String>, FilterOrderError> {
    if filters.is_empty() {
        return Ok(Vec::new());
    }

    let available: HashSet<&str> = filters.iter().map(|f| f.id).collect();
    let priority_map: HashMap<&str, (FilterPriority, usize)> = filters
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id, (f.priority, i)))
        .collect();

    // Build the dependency graph: edges[a] contains b means "a must come after b"
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for f in filters {
        in_degree.entry(f.id).or_insert(0);
    }

    // Process `after` constraints: f.after("dep") means f depends on dep
    for f in filters {
        for &dep in &f.after {
            if available.contains(dep) {
                *in_degree.entry(f.id).or_insert(0) += 1;
                dependents.entry(dep).or_default().push(f.id);
            }
        }
    }

    // Process `before` constraints: f.before("target") means target depends on f
    for f in filters {
        for &target in &f.before {
            if available.contains(target) {
                *in_degree.entry(target).or_insert(0) += 1;
                dependents.entry(f.id).or_default().push(target);
            }
        }
    }

    // Kahn's algorithm with priority-based tiebreaking
    let mut heap: BinaryHeap<PriorityEntry<'_>> = BinaryHeap::new();

    // Use the original filter order to seed the heap (deterministic)
    for f in filters {
        if in_degree.get(f.id) == Some(&0) {
            let (priority, insertion_order) = priority_map[f.id];
            heap.push(PriorityEntry {
                id: f.id,
                priority,
                insertion_order,
            });
        }
    }

    let mut sorted: Vec<String> = Vec::with_capacity(filters.len());

    while let Some(entry) = heap.pop() {
        sorted.push(entry.id.to_string());
        if let Some(deps) = dependents.get(entry.id) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        let (priority, insertion_order) = priority_map[dep];
                        heap.push(PriorityEntry {
                            id: dep,
                            priority,
                            insertion_order,
                        });
                    }
                }
            }
        }
    }

    if sorted.len() != filters.len() {
        let remaining: Vec<&str> = in_degree
            .iter()
            .filter(|(_, deg)| **deg > 0)
            .map(|(id, _)| *id)
            .collect();
        return Err(FilterOrderError::CyclicDependency(remaining.join(", ")));
    }

    Ok(sorted)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn meta(
        id: &'static str,
        priority: FilterPriority,
        after: Vec<&'static str>,
        before: Vec<&'static str>,
    ) -> FilterMetadata {
        FilterMetadata {
            id,
            priority,
            after,
            before,
        }
    }

    #[test]
    fn test_priority_ordering() {
        let filters = vec![
            meta("last", FilterPriority::Last, vec![], vec![]),
            meta("first", FilterPriority::First, vec![], vec![]),
            meta("normal", FilterPriority::Normal, vec![], vec![]),
        ];
        let order = resolve_filter_order(&filters).unwrap();
        assert_eq!(order, vec!["first", "normal", "last"]);
    }

    #[test]
    fn test_after_dependency() {
        let filters = vec![
            meta("a", FilterPriority::Normal, vec!["b"], vec![]),
            meta("b", FilterPriority::Normal, vec![], vec![]),
        ];
        let order = resolve_filter_order(&filters).unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        assert!(b_pos < a_pos, "b should execute before a");
    }

    #[test]
    fn test_before_dependency() {
        let filters = vec![
            meta("a", FilterPriority::Normal, vec![], vec!["b"]),
            meta("b", FilterPriority::Normal, vec![], vec![]),
        ];
        let order = resolve_filter_order(&filters).unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        assert!(a_pos < b_pos, "a should execute before b");
    }

    #[test]
    fn test_cycle_detection() {
        let filters = vec![
            meta("a", FilterPriority::Normal, vec!["b"], vec![]),
            meta("b", FilterPriority::Normal, vec!["a"], vec![]),
        ];
        let result = resolve_filter_order(&filters);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("circular dependency"));
    }

    #[test]
    fn test_priority_tiebreak() {
        // Same priority, no dependencies — insertion order wins
        let filters = vec![
            meta("a", FilterPriority::Normal, vec![], vec![]),
            meta("b", FilterPriority::Normal, vec![], vec![]),
        ];
        let order = resolve_filter_order(&filters).unwrap();
        assert_eq!(order, vec!["a", "b"]);
    }

    #[test]
    fn test_empty() {
        let order = resolve_filter_order(&[]).unwrap();
        assert!(order.is_empty());
    }

    #[test]
    fn test_unknown_dependency_ignored() {
        // after("nonexistent") should be silently ignored
        let filters = vec![meta(
            "a",
            FilterPriority::Normal,
            vec!["nonexistent"],
            vec![],
        )];
        let order = resolve_filter_order(&filters).unwrap();
        assert_eq!(order, vec!["a"]);
    }
}
