use std::collections::HashMap;
use std::sync::Arc;

use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_core::plugin::PluginContextFactoryImpl;
use infrarust_core::plugin::manager::PluginServices;

use crate::bindings::infrarust::plugin::config_service;
use crate::config::SandboxLimits;
use crate::store_state::{PluginStoreState, build_probe_state};

use super::{is_denied, services};

fn routed_state(capabilities: CapabilitySet) -> PluginStoreState {
    let router = Arc::new(infrarust_core::routing::DomainRouter::new());
    for (file, document) in [
        (
            "lobby.toml",
            "id = \"lobby\"\naddresses = [\"10.0.0.1:25565\"]\ndomains = [\"*.example.com\"]\n",
        ),
        (
            "survival.toml",
            "id = \"survival\"\naddresses = [\"10.0.0.2:25565\"]\ndomains = [\"survival.example.com\"]\n",
        ),
    ] {
        router.add(
            infrarust_core::provider::ProviderId::file(file),
            toml::from_str(document).unwrap(),
        );
    }
    let config_service = infrarust_core::services::config_service::ConfigServiceImpl::new(
        router,
        std::path::PathBuf::from("infrarust.toml"),
        Arc::new(toml::from_str::<infrarust_config::ProxyConfig>("").unwrap()),
    );
    let services = PluginServices {
        config_service: Arc::new(config_service),
        ..services(vec![])
    };
    build_probe_state("test".to_owned(), &SandboxLimits::default())
        .with_capabilities(capabilities)
        .with_ctx(
            PluginContextFactoryImpl::for_tests(services, HashMap::new()).create_context("test"),
        )
}

async fn server_for(state: &mut PluginStoreState, domain: &str) -> Option<String> {
    config_service::Host::get_server_by_domain(state, domain.into())
        .await
        .unwrap()
        .unwrap()
        .map(|config| config.id)
}

#[tokio::test]
async fn a_domain_finds_the_server_the_proxy_routes_it_to() {
    let mut state = routed_state(CapabilitySet::baseline());

    for (domain, server) in [
        ("survival.example.com", Some("survival")),
        ("Survival.Example.COM.", Some("survival")),
        ("hub.example.com", Some("lobby")),
        ("HUB.example.com.", Some("lobby")),
        ("example.com", None),
        ("survival.example.org", None),
    ] {
        assert_eq!(
            server_for(&mut state, domain).await.as_deref(),
            server,
            "{domain}"
        );
    }

    let survival =
        config_service::Host::get_server_by_domain(&mut state, "survival.example.com".into())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        survival,
        config_service::Host::get_server(&mut state, "survival".into())
            .await
            .unwrap()
            .unwrap(),
        "a domain answers the record get-server answers for its server"
    );
    assert_eq!(
        survival.map(|config| config.domains),
        Some(vec!["survival.example.com".to_owned()])
    );
}

#[tokio::test]
async fn a_domain_lookup_without_config_read_is_refused_like_get_server() {
    let mut state = routed_state(CapabilitySet::baseline().without(Capability::ConfigRead));
    let by_domain =
        config_service::Host::get_server_by_domain(&mut state, "survival.example.com".into())
            .await
            .unwrap();
    assert!(
        is_denied(&by_domain, Capability::ConfigRead),
        "{by_domain:?}"
    );
    assert_eq!(
        by_domain,
        config_service::Host::get_server(&mut state, "survival".into())
            .await
            .unwrap()
    );
}
