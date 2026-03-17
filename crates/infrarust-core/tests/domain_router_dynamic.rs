#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;

use infrarust_config::ServerConfig;
use infrarust_core::provider::ProviderId;
use infrarust_core::routing::DomainRouter;

/// Helper: build a minimal `ServerConfig` with the given domains.
fn make_config(domains: &[&str]) -> ServerConfig {
    toml::from_str(&format!(
        "domains = [{}]\naddresses = [\"127.0.0.1:25565\"]",
        domains
            .iter()
            .map(|d| format!("\"{d}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .unwrap()
}

#[test]
fn test_add_config_resolves() {
    let router = DomainRouter::new();
    let id = ProviderId::file("survival.toml");

    router.add(id.clone(), make_config(&["survival.mc.com"]));

    let (resolved_id, _cfg) = router.resolve("survival.mc.com").unwrap();
    assert_eq!(resolved_id, id);
}

#[test]
fn test_add_wildcard_resolves() {
    let router = DomainRouter::new();
    let id = ProviderId::file("survival.toml");

    router.add(id.clone(), make_config(&["*.survival.mc.com"]));

    let (resolved_id, _) = router.resolve("play.survival.mc.com").unwrap();
    assert_eq!(resolved_id, id);
}

#[test]
fn test_exact_match_priority_over_wildcard() {
    let router = DomainRouter::new();
    let wildcard_id = ProviderId::file("wildcard.toml");
    let exact_id = ProviderId::file("exact.toml");

    router.add(wildcard_id.clone(), make_config(&["*.mc.com"]));
    router.add(exact_id.clone(), make_config(&["specific.mc.com"]));

    // Exact match should win
    let (resolved_id, _) = router.resolve("specific.mc.com").unwrap();
    assert_eq!(resolved_id, exact_id);

    // Wildcard still works for other subdomains
    let (resolved_id, _) = router.resolve("other.mc.com").unwrap();
    assert_eq!(resolved_id, wildcard_id);
}

#[test]
fn test_update_config_changes_domains() {
    let router = DomainRouter::new();
    let id = ProviderId::file("survival.toml");

    router.add(id.clone(), make_config(&["old.mc.com"]));
    assert!(router.resolve("old.mc.com").is_some());

    // Update with different domain
    router.update(id, make_config(&["new.mc.com"]));
    assert!(router.resolve("old.mc.com").is_none());
    assert!(router.resolve("new.mc.com").is_some());
}

#[test]
fn test_remove_config_no_longer_resolves() {
    let router = DomainRouter::new();
    let id = ProviderId::file("survival.toml");

    router.add(id.clone(), make_config(&["survival.mc.com"]));
    assert!(router.resolve("survival.mc.com").is_some());

    router.remove(&id);
    assert!(router.resolve("survival.mc.com").is_none());
}

#[test]
fn test_remove_only_own_configs() {
    let router = DomainRouter::new();
    let id_file = ProviderId::file("survival.toml");
    let id_docker = ProviderId::docker("mc-creative-1");

    router.add(id_file, make_config(&["survival.mc.com"]));
    router.add(id_docker.clone(), make_config(&["creative.mc.com"]));

    router.remove(&id_docker);

    // File config should remain
    assert!(router.resolve("survival.mc.com").is_some());
    // Docker config should be gone
    assert!(router.resolve("creative.mc.com").is_none());
}

#[test]
fn test_remove_all_by_provider_type() {
    let router = DomainRouter::new();
    let id_file = ProviderId::file("survival.toml");
    let id_docker1 = ProviderId::docker("mc-1");
    let id_docker2 = ProviderId::docker("mc-2");

    router.add(id_file, make_config(&["survival.mc.com"]));
    router.add(id_docker1, make_config(&["creative.mc.com"]));
    router.add(id_docker2, make_config(&["minigames.mc.com"]));

    assert_eq!(router.len(), 3);

    router.remove_all_by_provider_type("docker");

    assert_eq!(router.len(), 1);
    assert!(router.resolve("survival.mc.com").is_some());
    assert!(router.resolve("creative.mc.com").is_none());
    assert!(router.resolve("minigames.mc.com").is_none());
}

#[test]
fn test_list_all_returns_all_with_ids() {
    let router = DomainRouter::new();
    let id_file = ProviderId::file("a.toml");
    let id_docker = ProviderId::docker("mc-1");

    router.add(id_file.clone(), make_config(&["a.com"]));
    router.add(id_docker.clone(), make_config(&["b.com"]));

    let all = router.list_all();
    assert_eq!(all.len(), 2);

    let ids: Vec<ProviderId> = all.iter().map(|(id, _)| id.clone()).collect();
    assert!(ids.contains(&id_file));
    assert!(ids.contains(&id_docker));
}

#[test]
fn test_count_by_provider() {
    let router = DomainRouter::new();

    router.add(ProviderId::file("a.toml"), make_config(&["a.com"]));
    router.add(ProviderId::file("b.toml"), make_config(&["b.com"]));
    router.add(ProviderId::file("c.toml"), make_config(&["c.com"]));
    router.add(ProviderId::docker("mc-1"), make_config(&["d.com"]));
    router.add(ProviderId::docker("mc-2"), make_config(&["e.com"]));

    let counts = router.count_by_provider();
    assert_eq!(counts.get("file"), Some(&3));
    assert_eq!(counts.get("docker"), Some(&2));
}

#[test]
fn test_resolve_case_insensitive() {
    let router = DomainRouter::new();
    let id = ProviderId::file("test.toml");

    router.add(id, make_config(&["Test.MC.Com"]));

    assert!(router.resolve("test.mc.com").is_some());
    assert!(router.resolve("TEST.MC.COM").is_some());
}

#[test]
fn test_resolve_strips_fml_marker() {
    let router = DomainRouter::new();
    let id = ProviderId::file("test.toml");

    router.add(id.clone(), make_config(&["mc.example.com"]));

    let (resolved_id, _) = router.resolve("mc.example.com\0FML2").unwrap();
    assert_eq!(resolved_id, id);
}

#[test]
fn test_empty_router_resolves_none() {
    let router = DomainRouter::new();
    assert!(router.resolve("anything.com").is_none());
    assert!(router.is_empty());
    assert_eq!(router.len(), 0);
}

#[test]
fn test_remove_wildcard_patterns() {
    let router = DomainRouter::new();
    let id = ProviderId::file("wild.toml");

    router.add(id.clone(), make_config(&["*.example.com"]));
    assert!(router.resolve("foo.example.com").is_some());

    router.remove(&id);
    assert!(router.resolve("foo.example.com").is_none());
}

#[test]
fn test_domain_conflict_last_write_wins() {
    let router = DomainRouter::new();
    let id_a = ProviderId::file("a.toml");
    let id_b = ProviderId::file("b.toml");

    router.add(id_a, make_config(&["shared.mc.com"]));
    router.add(id_b.clone(), make_config(&["shared.mc.com"]));

    // Last writer (b) should own the domain
    let (resolved_id, _) = router.resolve("shared.mc.com").unwrap();
    assert_eq!(resolved_id, id_b);
}

#[test]
fn test_count_by_provider_empty() {
    let router = DomainRouter::new();
    let counts: HashMap<String, usize> = router.count_by_provider();
    assert!(counts.is_empty());
}
