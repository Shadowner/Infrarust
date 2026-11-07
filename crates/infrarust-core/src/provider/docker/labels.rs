//! Docker label parsing and address resolution.
//!
//! Converts Docker container labels (`infrarust.*`) to `ServerConfig`
//! and resolves container network addresses.

use std::collections::HashMap;

use bollard::models::ContainerInspectResponse;
use wildmatch::WildMatch;

use infrarust_config::{
    DockerProviderConfig, ForwardingMode, ProxyMode, ServerAddress, ServerConfig,
};

/// Default Minecraft port.
/// Converts Docker container labels to a `ServerConfig`.
pub fn labels_to_server_config(
    container_name: &str,
    labels: &HashMap<String, String>,
    address: &str,
) -> ServerConfig {
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

    let name = labels.get("infrarust.name").cloned();
    let network = labels.get("infrarust.network").cloned();

    let send_proxy_protocol = labels
        .get("infrarust.send_proxy_protocol")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let server_address: ServerAddress = address.parse().unwrap_or_else(|_| ServerAddress {
        host: address.to_string(),
        port: infrarust_config::DEFAULT_MC_PORT,
    });

    // Deserializing through a toml::Table (instead of a struct literal) keeps
    // serde defaults for all unset fields and escapes label values correctly.
    let built = (|| -> Result<ServerConfig, Box<dyn std::error::Error>> {
        use toml::{Table, Value};

        let mut table = Table::new();
        table.insert("id".into(), Value::String(container_name.to_string()));
        table.insert(
            "domains".into(),
            Value::Array(domains.iter().cloned().map(Value::String).collect()),
        );
        table.insert(
            "addresses".into(),
            Value::Array(vec![Value::String(server_address.to_string())]),
        );
        table.insert("proxy_mode".into(), Value::try_from(proxy_mode)?);
        table.insert(
            "send_proxy_protocol".into(),
            Value::Boolean(send_proxy_protocol),
        );
        if let Some(name) = &name {
            table.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(network) = &network {
            table.insert("network".into(), Value::String(network.clone()));
        }
        if let Some(text) = labels.get("infrarust.motd.text") {
            let mut online = Table::new();
            online.insert("text".into(), Value::String(text.clone()));
            let mut motd = Table::new();
            motd.insert("online".into(), Value::Table(online));
            table.insert("motd".into(), Value::Table(motd));
        }

        Ok(table.try_into()?)
    })();

    built.unwrap_or_else(|e| {
        tracing::warn!(
            container = %container_name,
            error = %e,
            "generated docker server config failed to parse, using minimal fallback"
        );
        // Manual fallback — less complete but functional
        ServerConfig {
            id: Some(container_name.to_string()),
            name,
            network,
            domains,
            addresses: vec![server_address.into()],
            balance: Default::default(),
            slow_start: None,
            slow_start_aggression: 1.0,
            active_health: None,
            proxy_mode,
            forwarding_mode: None,
            send_proxy_protocol,
            domain_rewrite: Default::default(),
            motd: Default::default(),
            server_manager: None,
            timeouts: None,
            max_players: 0,
            ip_filter: None,
            disconnect_message: None,
            limbo_handlers: Vec::new(),
            bungeecord_channel: false,
        }
    })
}

/// A container whose labels claim domains the `[docker]` section does not allow.
#[derive(Debug, PartialEq, Eq)]
pub struct DomainsNotAllowed {
    /// The complete `allowed_domains` line that would admit this container.
    pub allowing_line: String,
}

/// Builds the config of a container under the `[docker]` policy.
pub fn container_server_config(
    policy: &DockerProviderConfig,
    container_name: &str,
    labels: &HashMap<String, String>,
    address: &str,
) -> Result<ServerConfig, DomainsNotAllowed> {
    let mut config = labels_to_server_config(container_name, labels, address);

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
        return Err(DomainsNotAllowed {
            allowing_line: format!("allowed_domains = {:?}", [allowed, refused].concat()),
        });
    }

    if !policy.inherit_forwarding {
        config.forwarding_mode = Some(ForwardingMode::None);
    }
    Ok(config)
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
/// 2. Port bindings → `host_ip:host_port`
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

    // 2. Port bindings
    if let Some(bindings) = info
        .host_config
        .as_ref()
        .and_then(|hc| hc.port_bindings.as_ref())
    {
        let key = format!("{port}/tcp");
        if let Some(Some(binding_list)) = bindings.get(&key)
            && let Some(binding) = binding_list.first()
        {
            let host_port = binding.host_port.as_deref().unwrap_or("25565");
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
