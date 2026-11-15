//! Docker label parsing and address resolution.
//!
//! Converts Docker container labels (`infrarust.*`) to `ServerConfig`
//! and resolves container network addresses.

use std::collections::HashMap;

use bollard::models::ContainerInspectResponse;
use toml::{Table, Value};
use wildmatch::WildMatch;

use infrarust_config::{
    DockerProviderConfig, ForwardingMode, ProxyMode, ServerAddress, ServerConfig,
};

pub fn labels_to_server_config(
    container_name: &str,
    labels: &HashMap<String, String>,
    address: &str,
) -> Result<ServerConfig, toml::de::Error> {
    let domains = labels
        .get("infrarust.domains")
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_else(|| vec![format!("{container_name}.docker.local")]);

    let proxy_mode = labels
        .get("infrarust.proxy_mode")
        .and_then(|m| match m.as_str() {
            "passthrough" => Some(ProxyMode::Passthrough),
            "client_only" => Some(ProxyMode::ClientOnly),
            "offline" => Some(ProxyMode::Offline),
            "server_only" => Some(ProxyMode::ServerOnly),
            "zero_copy" => Some(ProxyMode::ZeroCopy),
            _ => None,
        })
        .unwrap_or(ProxyMode::Passthrough);

    let send_proxy_protocol = labels
        .get("infrarust.send_proxy_protocol")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let server_address: ServerAddress = address.parse().unwrap_or_else(|_| ServerAddress {
        host: address.to_string(),
        port: infrarust_config::DEFAULT_MC_PORT,
    });

    let mut table = Table::new();
    table.insert("id".into(), Value::String(container_name.to_string()));
    table.insert(
        "domains".into(),
        Value::Array(domains.into_iter().map(Value::String).collect()),
    );
    table.insert(
        "addresses".into(),
        Value::Array(vec![Value::String(server_address.to_string())]),
    );
    table.insert(
        "proxy_mode".into(),
        Value::try_from(proxy_mode).map_err(serde::de::Error::custom)?,
    );
    table.insert(
        "send_proxy_protocol".into(),
        Value::Boolean(send_proxy_protocol),
    );
    for key in ["name", "network"] {
        if let Some(value) = labels.get(&format!("infrarust.{key}")) {
            table.insert(key.into(), Value::String(value.clone()));
        }
    }
    if let Some(text) = labels.get("infrarust.motd.text") {
        let mut online = Table::new();
        online.insert("text".into(), Value::String(text.clone()));
        let mut motd = Table::new();
        motd.insert("online".into(), Value::Table(online));
        table.insert("motd".into(), Value::Table(motd));
    }

    table.try_into()
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContainerRefused {
    DomainsNotAllowed { allowing_line: String },
    InvalidLabels(toml::de::Error),
}

/// Builds the config of a container under the `[docker]` policy.
pub fn container_server_config(
    policy: &DockerProviderConfig,
    container_name: &str,
    labels: &HashMap<String, String>,
    address: &str,
) -> Result<ServerConfig, ContainerRefused> {
    let mut config = labels_to_server_config(container_name, labels, address)
        .map_err(ContainerRefused::InvalidLabels)?;

    let own_domain = format!("{container_name}.docker.local").to_lowercase();
    let allowed: Vec<String> = policy
        .allowed_domains
        .iter()
        .map(|domain| domain.to_lowercase())
        .collect();
    let mut refused: Vec<String> = config
        .domains
        .iter()
        .map(|domain| domain.to_lowercase())
        .filter(|domain| *domain != own_domain && !domain_allowed(&allowed, domain))
        .collect();
    if !refused.is_empty() {
        refused.dedup();
        return Err(ContainerRefused::DomainsNotAllowed {
            allowing_line: format!("allowed_domains = {:?}", [allowed, refused].concat()),
        });
    }

    if !(policy.inherit_forwarding && config.proxy_mode.is_intercepted()) {
        config.forwarding_mode = Some(ForwardingMode::None);
    }
    Ok(config)
}

/// The warning owed when a container does not get the proxy-wide forwarding
/// mode its operator may expect it to have.
pub fn withheld_forwarding_warning(
    policy: &DockerProviderConfig,
    proxy_wide: &ForwardingMode,
    container_name: &str,
    config: &ServerConfig,
) -> Option<String> {
    if *proxy_wide == ForwardingMode::None {
        return None;
    }
    match (
        policy.inherit_forwarding,
        config.proxy_mode.is_intercepted(),
    ) {
        (false, true) => Some(format!(
            "container {container_name} and any other container on an intercepted proxy mode \
             get no player forwarding, although [forwarding] selects {proxy_wide:?}: a backend \
             that expects it rejects every player. To hand containers the mode and its secret, \
             set inherit_forwarding = true in [docker]"
        )),
        (true, false) => Some(format!(
            "container {container_name} and any other container on {:?} or another proxy mode \
             that relays logins untouched get no player forwarding, although [docker] sets \
             inherit_forwarding = true and [forwarding] selects {proxy_wide:?}: the proxy \
             cannot vouch for a player it does not log in. Only containers labelled \
             infrarust.proxy_mode=client_only or offline inherit the mode",
            config.proxy_mode
        )),
        (false, false) | (true, true) => None,
    }
}

fn domain_allowed(allowed: &[String], domain: &str) -> bool {
    let is_pattern = domain.contains(['*', '?']);
    allowed
        .iter()
        .any(|entry| entry == domain || (!is_pattern && WildMatch::new(entry).matches(domain)))
}

/// Resolves the best address for a Docker container.
///
/// Priority:
/// 1. Network IP from the preferred network (or first available)
/// 2. Ports the daemon published → `host_ip:host_port`
pub fn resolve_container_address(
    info: &ContainerInspectResponse,
    preferred_network: Option<&str>,
    port: u16,
) -> Option<String> {
    // 1. Network IP
    if let Some(networks) = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.networks.as_ref())
    {
        // Try preferred network first
        if let Some(net_name) = preferred_network
            && let Some(net) = networks.get(net_name)
            && let Some(ip) = &net.ip_address
            && !ip.is_empty()
        {
            return Some(format!("{ip}:{port}"));
        }
        let first_by_name = networks
            .iter()
            .filter_map(|(name, net)| {
                let ip = net.ip_address.as_deref().filter(|ip| !ip.is_empty())?;
                Some((name, ip))
            })
            .min();
        if let Some((_, ip)) = first_by_name {
            return Some(format!("{ip}:{port}"));
        }
    }

    // 2. Published ports
    if let Some(ports) = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.ports.as_ref())
    {
        let key = format!("{port}/tcp");
        if let Some(Some(binding_list)) = ports.get(&key)
            && let Some(binding) = binding_list.first()
            && let Some(host_port) = binding.host_port.as_deref()
        {
            let host_ip = binding.host_ip.as_deref().unwrap_or("0.0.0.0");
            let actual_ip = if host_ip == "0.0.0.0" {
                "127.0.0.1"
            } else {
                host_ip
            };
            return Some(format!("{actual_ip}:{host_port}"));
        }
    }

    None
}
