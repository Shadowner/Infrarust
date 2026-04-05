//! Docker label parsing and address resolution.
//!
//! Converts Docker container labels (`infrarust.*`) to `ServerConfig`
//! and resolves container network addresses.

use std::collections::HashMap;

use bollard::models::ContainerInspectResponse;

use infrarust_config::{ProxyMode, ServerAddress, ServerConfig};

/// Default Minecraft port.
const DEFAULT_MC_PORT: u16 = 25565;

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
        port: DEFAULT_MC_PORT,
    });

    // Build a TOML string and parse it to satisfy deny_unknown_fields
    // This ensures we get proper defaults for all fields.
    let mut toml_str = format!(
        "domains = [{}]\naddresses = [\"{}\"]\nproxy_mode = \"{:?}\"\nsend_proxy_protocol = {}",
        domains
            .iter()
            .map(|d| format!("\"{}\"", d))
            .collect::<Vec<_>>()
            .join(", "),
        server_address,
        proxy_mode,
        send_proxy_protocol,
    );

    if let Some(ref name) = name {
        toml_str.push_str(&format!("\nname = \"{name}\""));
    }

    if let Some(ref network) = network {
        toml_str.push_str(&format!("\nnetwork = \"{network}\""));
    }

    // Add MOTD text if specified
    if let Some(motd_text) = labels.get("infrarust.motd.text") {
        toml_str.push_str(&format!("\n\n[motd.online]\ntext = \"{}\"", motd_text));
    }

    // Try to parse, fall back to manual construction
    toml::from_str::<ServerConfig>(&toml_str).unwrap_or_else(|_| {
        // Manual fallback — less complete but functional
        ServerConfig {
            id: Some(container_name.to_string()),
            name,
            network,
            domains,
            addresses: vec![server_address],
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
        }
    })
}

/// Resolves the best address for a Docker container.
///
/// Priority:
/// 1. Network IP from the preferred network (or first available)
/// 2. Port bindings → `host_ip:host_port`
/// 3. Container name as hostname
pub fn resolve_container_address(
    info: &ContainerInspectResponse,
    preferred_network: Option<&str>,
    port: u16,
) -> String {
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
            return format!("{ip}:{port}");
        }
        // Try any network
        for net in networks.values() {
            if let Some(ip) = &net.ip_address
                && !ip.is_empty()
            {
                return format!("{ip}:{port}");
            }
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
            return format!("{actual_ip}:{host_port}");
        }
    }

    // 3. Container name
    let name = info
        .name
        .as_deref()
        .unwrap_or("unknown")
        .trim_start_matches('/');
    format!("{name}:{port}")
}
