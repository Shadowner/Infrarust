use infrarust_config::models::server::{MotdConfig, ProxyModeEnum};
use infrarust_protocol::{
    minecraft::java::status::clientbound_response::{
        CLIENTBOUND_RESPONSE_ID, ClientBoundResponse, PlayersJSON, ResponseJSON, VersionJSON,
    },
    types::ProtocolString,
};
use std::sync::Arc;
use tracing::debug;

use crate::{
    InfrarustConfig,
    core::config::ServerConfig,
    network::{
        packet::{Packet, PacketCodec},
        proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    },
    server::ServerResponse,
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

pub fn generate_motd(
    motd: &MotdConfig,
    include_infrarust_favicon: bool,
) -> Result<Packet, ProxyProtocolError> {
    let status_json = ResponseJSON {
        version: VersionJSON {
            name: motd.version_name.clone().unwrap_or_default(),
            protocol: motd.protocol_version.unwrap_or_default(),
        },
        players: PlayersJSON {
            max: motd.max_players.unwrap_or_default(),
            online: motd.online_players.unwrap_or_default(),
            sample: motd.samples.clone().unwrap_or_default(),
        },
        description: serde_json::json!({
            "text":  motd.text.clone().unwrap_or_default(),
        }),
        favicon: motd.favicon.clone().or_else(|| {
            if include_infrarust_favicon {
                Some(FAVICON.to_string())
            } else {
                None
            }
        }),
        previews_chat: false,
        enforces_secure_chat: false,
        modinfo: None,
        forge_data: None,
    };

    let json_str = match serde_json::to_string(&status_json) {
        Ok(json_str) => json_str,
        Err(e) => {
            #[cfg(feature = "telemetry")]
            TELEMETRY.record_internal_error("status_json_serialize_failed", None, None);

            return Err(ProxyProtocolError::Other(format!(
                "Failed to serialize status JSON: {}",
                e
            )));
        }
    };

    let mut response_packet = Packet::new(CLIENTBOUND_RESPONSE_ID);
    response_packet.encode(&ClientBoundResponse {
        json_response: ProtocolString(json_str),
    })?;

    Ok(response_packet)
}

pub fn generate_unreachable_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
    config: &InfrarustConfig,
) -> ProtocolResult<ServerResponse> {
    let motd_packet = if let Some(motd) = &server.motd {
        generate_motd(motd, false)?
    } else if let Some(motd) = config.motds.unreachable.clone() {
        generate_motd(&motd, true)?
    } else {
        generate_motd(&MotdConfig::default_unreachable(), true)?
    };

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_unknown_server_response(
    domain: String,
    config: &InfrarustConfig,
) -> ProtocolResult<ServerResponse> {
    let fake_config = Arc::new(ServerConfig {
        domains: vec![domain.clone()],
        addresses: vec![],
        config_id: format!("unknown_{}", domain),
        ..ServerConfig::default()
    });

    if let Some(motd) = config.motds.unknown.clone() {
        let motd_packet = generate_motd(&motd, true)?;

        Ok(ServerResponse {
            server_conn: None,
            status_response: Some(motd_packet),
            send_proxy_protocol: false,
            read_packets: vec![],
            server_addr: None,
            proxy_mode: ProxyModeEnum::Status,
            proxied_domain: Some(domain),
            initial_config: fake_config,
        })
    } else {
        Err(ProxyProtocolError::Other(format!(
            "Server not found for domain: {}",
            domain
        )))
    }
}

pub fn generate_starting_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some("§6Server is starting...§r\n§8§oPlease wait a moment".to_string()),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_not_started_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some("§e§oServer is sleeping. §8§o\nConnect to it to wake it up.".to_string()),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };
    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_unable_status_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some(
            "§cUnable to obtain server status...§r\n§8§o -> Contact an admin if the issue persist."
                .to_string(),
        ),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_crashing_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some(
            "§4Server is in a crashing state...§r\n§8§o -> Contact an admin if the issue persist."
                .to_string(),
        ),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_unknown_status_server_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some(
            "§cUnknown server status...§r\n§8§o -> Contact an admin if the issue persist."
                .to_string(),
        ),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_stopping_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd = MotdConfig {
        text: Some(
            "§6Server is marked to shutdown...\n§8§o Connect to it to cancel it !".to_string(),
        ),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

pub fn generate_imminent_shutdown_motd_response(
    domain: String,
    server: Arc<ServerConfig>,
    seconds_remaining: u64,
) -> ProtocolResult<ServerResponse> {
    let time_str = if seconds_remaining <= 60 {
        format!("{} seconds", seconds_remaining)
    } else {
        format!("{:.1} minutes", seconds_remaining as f64 / 60.0)
    };

    let motd = MotdConfig {
        text: Some(format!(
            "§c§lServer shutting down in {}!§r\n§e§oConnect now to keep it online!",
            time_str
        )),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    };

    let motd_packet = generate_motd(&motd, true)?;

    Ok(ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    })
}

const FAVICON: &str = "data:image/png;base64,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";

pub async fn handle_server_fetch_error(
    server_config: &crate::core::config::ServerConfig,
    domain: &str,
    motd_config: &MotdConfig,
) -> ProtocolResult<Packet> {
    debug!("Generating fallback MOTD for {}", domain);

    if let Some(motd) = &server_config.motd {
        debug!("Using server-specific MOTD for {}", domain);
        return generate_motd(motd, false);
    }

    if motd_config.enabled {
        if !motd_config.is_empty() {
            debug!("Using global 'unreachable' MOTD");
            return generate_motd(motd_config, true);
        }
        debug!("Using default 'unreachable' MOTD");
        return generate_motd(&MotdConfig::default_unreachable(), true);
    }

    Err(ProxyProtocolError::Other(format!(
        "Failed to connect to server for domain: {}",
        domain
    )))
}

pub async fn handle_server_fetch_error_with_shared(
    server_config: &crate::core::config::ServerConfig,
    domain: &str,
    shared_component: &Arc<crate::core::shared_component::SharedComponent>,
) -> ProtocolResult<Packet> {
    let motd_config = if let Some(config) = &shared_component.config().motds.unreachable {
        config.clone()
    } else {
        MotdConfig::default()
    };

    handle_server_fetch_error(server_config, domain, &motd_config).await
}
