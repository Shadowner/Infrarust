use std::sync::Arc;

use infrarust_config::{
    ServerConfig,
    models::{logging::LogType, server::MotdConfig},
};
use tracing::debug;

use crate::{
    InfrarustConfig,
    network::{
        packet::Packet,
        proxy_protocol::{ProtocolResult, errors::ProxyProtocolError},
    },
    server::ServerResponse,
};

use super::{
    generator::{generate_for_state, generate_motd_packet, get_motd_config_for_state},
    state::MotdState,
};

fn create_server_response(
    domain: Arc<str>,
    server: Arc<ServerConfig>,
    motd_packet: Packet,
) -> ServerResponse {
    use infrarust_config::models::server::ProxyModeEnum;

    ServerResponse {
        server_conn: None,
        status_response: Some(motd_packet),
        send_proxy_protocol: false,
        read_packets: vec![],
        server_addr: None,
        proxy_mode: ProxyModeEnum::Status,
        proxied_domain: Some(domain),
        initial_config: server,
    }
}

pub fn generate_response(
    state: MotdState,
    domain: Arc<str>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    let motd_config = get_motd_config_for_state(&state, &server.motds);
    let motd_packet = generate_for_state(&state, motd_config)?;
    Ok(create_server_response(domain, server, motd_packet))
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Unreachable instead"
)]
pub fn generate_unreachable_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
    config: &InfrarustConfig,
) -> ProtocolResult<ServerResponse> {
    let motd_packet = if let Some(motd) = &server.motds.unreachable {
        generate_motd_packet(motd, false)?
    } else if let Some(motd) = config.motds.unreachable.clone() {
        generate_motd_packet(&motd, true)?
    } else {
        generate_motd_packet(&MotdConfig::default_unreachable(), true)?
    };
    Ok(create_server_response(
        Arc::from(domain.into()),
        server,
        motd_packet,
    ))
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::UnknownServer instead"
)]
pub fn generate_unknown_server_response(
    domain: impl Into<String> + std::fmt::Display + Clone,
    config: &InfrarustConfig,
) -> ProtocolResult<ServerResponse> {
    let domain_str = domain.to_string();
    let fake_config = Arc::new(ServerConfig {
        domains: vec![domain_str.clone()],
        addresses: vec![],
        config_id: format!("unknown_{}", domain_str),
        ..ServerConfig::default()
    });

    if let Some(motd) = config.motds.unknown.clone() {
        let motd_packet = generate_motd_packet(&motd, true)?;
        Ok(create_server_response(
            Arc::from(domain_str),
            fake_config,
            motd_packet,
        ))
    } else {
        Err(ProxyProtocolError::Other(format!(
            "Server not found for domain: {}",
            domain_str
        )))
    }
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Starting instead"
)]
pub fn generate_starting_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Starting, Arc::from(domain.into()), server)
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Offline instead"
)]
pub fn generate_not_started_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Offline, Arc::from(domain.into()), server)
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::UnableToFetchStatus instead"
)]
pub fn generate_unable_status_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(
        MotdState::UnableToFetchStatus,
        Arc::from(domain.into()),
        server,
    )
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Crashed instead"
)]
pub fn generate_crashing_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Crashed, Arc::from(domain.into()), server)
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Unknown instead"
)]
pub fn generate_unknown_status_server_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Unknown, Arc::from(domain.into()), server)
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Stopping instead"
)]
pub fn generate_stopping_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Stopping, Arc::from(domain.into()), server)
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::ImminentShutdown instead"
)]
pub fn generate_imminent_shutdown_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
    seconds_remaining: u64,
) -> ProtocolResult<ServerResponse> {
    generate_response(
        MotdState::ImminentShutdown { seconds_remaining },
        Arc::from(domain.into()),
        server,
    )
}

#[deprecated(
    since = "1.5.1",
    note = "Use generate_response with MotdState::Online instead"
)]
pub fn generate_online_motd_response(
    domain: impl Into<String>,
    server: Arc<ServerConfig>,
) -> ProtocolResult<ServerResponse> {
    generate_response(MotdState::Online, Arc::from(domain.into()), server)
}

pub async fn handle_server_fetch_error(
    server_config: &ServerConfig,
    domain: &str,
    motd_config: &MotdConfig,
) -> ProtocolResult<Packet> {
    debug!(
        log_type = LogType::Motd.as_str(),
        "Generating fallback MOTD for {}", domain
    );

    if let Some(motd) = &server_config.motds.online {
        debug!(
            log_type = LogType::Motd.as_str(),
            "Using server-specific MOTD for {}", domain
        );
        return generate_motd_packet(motd, false);
    }

    if motd_config.enabled {
        if !motd_config.is_empty() {
            debug!(
                log_type = LogType::Motd.as_str(),
                "Using global 'unreachable' MOTD"
            );
            return generate_motd_packet(motd_config, true);
        }
        debug!(
            log_type = LogType::Motd.as_str(),
            "Using default 'unreachable' MOTD"
        );
        return generate_motd_packet(&MotdConfig::default_unreachable(), true);
    }

    Err(ProxyProtocolError::Other(format!(
        "Failed to connect to server for domain: {}",
        domain
    )))
}

pub async fn handle_server_fetch_error_with_shared(
    server_config: &ServerConfig,
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
