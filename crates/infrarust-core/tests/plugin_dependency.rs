#![allow(clippy::unwrap_used)]

use infrarust_api::plugin::PluginMetadata;
use infrarust_core::plugin::dependency::resolve_load_order;

#[test]
fn test_no_dependencies() {
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0"),
        PluginMetadata::new("b", "B", "1.0"),
        PluginMetadata::new("c", "C", "1.0"),
    ];
    let order = resolve_load_order(&plugins).unwrap();
    assert_eq!(order.len(), 3);
    assert!(order.contains(&"a".to_string()));
    assert!(order.contains(&"b".to_string()));
    assert!(order.contains(&"c".to_string()));
}

#[test]
fn test_simple_dependency() {
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0").depends_on("b"),
        PluginMetadata::new("b", "B", "1.0"),
    ];
    let order = resolve_load_order(&plugins).unwrap();
    let pos_a = order.iter().position(|x| x == "a").unwrap();
    let pos_b = order.iter().position(|x| x == "b").unwrap();
    assert!(pos_b < pos_a, "B must load before A");
}

#[test]
fn test_chain() {
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0").depends_on("b"),
        PluginMetadata::new("b", "B", "1.0").depends_on("c"),
        PluginMetadata::new("c", "C", "1.0"),
    ];
    let order = resolve_load_order(&plugins).unwrap();
    assert_eq!(order, vec!["c", "b", "a"]);
}

#[test]
fn test_missing_required() {
    let plugins = vec![PluginMetadata::new("a", "A", "1.0").depends_on("missing")];
    let result = resolve_load_order(&plugins);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("missing"),
        "Error should mention the missing dep: {err}"
    );
}

#[test]
fn test_missing_optional() {
    let plugins = vec![PluginMetadata::new("a", "A", "1.0").optional_dependency("missing")];
    let order = resolve_load_order(&plugins).unwrap();
    assert_eq!(order, vec!["a"]);
}

#[test]
fn test_optional_present_influences_order() {
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0").optional_dependency("b"),
        PluginMetadata::new("b", "B", "1.0"),
    ];
    let order = resolve_load_order(&plugins).unwrap();
    let pos_a = order.iter().position(|x| x == "a").unwrap();
    let pos_b = order.iter().position(|x| x == "b").unwrap();
    assert!(
        pos_b < pos_a,
        "Optional dep B should load before A when present"
    );
}

#[test]
fn test_cycle() {
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0").depends_on("b"),
        PluginMetadata::new("b", "B", "1.0").depends_on("a"),
    ];
    let result = resolve_load_order(&plugins);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Circular dependency"),
        "Error should mention circular dependency: {err}"
    );
}

#[test]
fn test_diamond() {
    // A -> B, A -> C, B -> D, C -> D
    let plugins = vec![
        PluginMetadata::new("a", "A", "1.0")
            .depends_on("b")
            .depends_on("c"),
        PluginMetadata::new("b", "B", "1.0").depends_on("d"),
        PluginMetadata::new("c", "C", "1.0").depends_on("d"),
        PluginMetadata::new("d", "D", "1.0"),
    ];
    let order = resolve_load_order(&plugins).unwrap();
    let pos = |id: &str| order.iter().position(|x| x == id).unwrap();
    assert!(pos("d") < pos("b"), "D before B");
    assert!(pos("d") < pos("c"), "D before C");
    assert!(pos("b") < pos("a"), "B before A");
    assert!(pos("c") < pos("a"), "C before A");
}
