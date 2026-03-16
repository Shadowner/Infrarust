use std::collections::HashMap;

use infrarust_core::provider::ProviderId;

#[test]
fn test_provider_id_display() {
    let id = ProviderId::file("survival.toml");
    assert_eq!(id.to_string(), "file@survival.toml");

    let id = ProviderId::docker("mc-survival-1");
    assert_eq!(id.to_string(), "docker@mc-survival-1");
}

#[test]
fn test_provider_id_parse() {
    let id: ProviderId = "docker@mc-1".parse().unwrap();
    assert_eq!(id.provider_type, "docker");
    assert_eq!(id.unique_id, "mc-1");
}

#[test]
fn test_provider_id_parse_with_multiple_at() {
    // Only split on the first '@'
    let id: ProviderId = "file@config@special.toml".parse().unwrap();
    assert_eq!(id.provider_type, "file");
    assert_eq!(id.unique_id, "config@special.toml");
}

#[test]
fn test_provider_id_parse_invalid() {
    let result = "no_at_sign".parse::<ProviderId>();
    assert!(result.is_err());
}

#[test]
fn test_provider_id_equality() {
    let a = ProviderId::file("survival.toml");
    let b = ProviderId::file("survival.toml");
    assert_eq!(a, b);

    let c = ProviderId::docker("survival.toml");
    assert_ne!(a, c); // Different provider type
}

#[test]
fn test_provider_id_hash() {
    let mut map = HashMap::new();
    let id = ProviderId::file("survival.toml");
    map.insert(id.clone(), 42);
    assert_eq!(map.get(&id), Some(&42));

    // Different id should not be found
    let other = ProviderId::file("creative.toml");
    assert_eq!(map.get(&other), None);
}

#[test]
fn test_provider_id_roundtrip() {
    let original = ProviderId::new("api", "admin-created-1");
    let s = original.to_string();
    let parsed: ProviderId = s.parse().unwrap();
    assert_eq!(original, parsed);
}
