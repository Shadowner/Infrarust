use std::io;

use infrarust_config::models::server::MotdConfig;
use infrarust_protocol::{
    minecraft::java::{
        legacy::{
            kick::{build_legacy_kick_beta, build_legacy_kick_v1_4},
            ping::LegacyPingVariant,
        },
        status::clientbound_response::{
            CLIENTBOUND_RESPONSE_ID, ClientBoundResponse, PlayersJSON, ResponseJSON, VersionJSON,
        },
    },
    types::ProtocolString,
};

use crate::network::{
    packet::{Packet, PacketCodec},
    proxy_protocol::errors::ProxyProtocolError,
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

use super::{
    favicon::{get_default_favicon, parse_favicon},
    state::MotdState,
};

pub fn generate_motd_packet(
    motd: &MotdConfig,
    include_infrarust_favicon: bool,
) -> Result<Packet, ProxyProtocolError> {
    let status_json = ResponseJSON {
        version: VersionJSON {
            name: motd.version_name.as_deref().unwrap_or_default().to_string(),
            protocol: motd.protocol_version.unwrap_or_default(),
        },
        players: PlayersJSON {
            max: motd.max_players.unwrap_or_default(),
            online: motd.online_players.unwrap_or_default(),
            sample: motd.samples.as_ref().cloned().unwrap_or_default(),
        },
        description: serde_json::json!({
            "text": motd.text.as_deref().unwrap_or_default(),
        }),
        favicon: motd
            .favicon
            .as_ref()
            .and_then(|f| parse_favicon(f))
            .or_else(|| get_default_favicon(include_infrarust_favicon)),
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

fn generate_motd_packet_with_text(
    motd: &MotdConfig,
    text_override: &str,
    include_infrarust_favicon: bool,
) -> Result<Packet, ProxyProtocolError> {
    let status_json = ResponseJSON {
        version: VersionJSON {
            name: motd.version_name.as_deref().unwrap_or_default().to_string(),
            protocol: motd.protocol_version.unwrap_or_default(),
        },
        players: PlayersJSON {
            max: motd.max_players.unwrap_or_default(),
            online: motd.online_players.unwrap_or_default(),
            sample: motd.samples.as_ref().cloned().unwrap_or_default(),
        },
        description: serde_json::json!({ "text": text_override }),
        favicon: motd
            .favicon
            .as_ref()
            .and_then(|f| parse_favicon(f))
            .or_else(|| get_default_favicon(include_infrarust_favicon)),
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

fn create_default_motd(text: String) -> MotdConfig {
    MotdConfig {
        text: Some(text),
        version_name: Some("Infrarust".to_string()),
        max_players: Some(0),
        online_players: Some(0),
        protocol_version: Some(0),
        samples: Some(Vec::new()),
        ..Default::default()
    }
}

pub fn generate_for_state(
    state: &MotdState,
    motd_config: Option<&MotdConfig>,
) -> Result<Packet, ProxyProtocolError> {
    let use_default_favicon = motd_config.is_none() || state.use_default_favicon();

    match (state, motd_config) {
        // If a custom MOTD config is provided, use it
        (MotdState::ImminentShutdown { seconds_remaining }, Some(motd)) => {
            // Special handling for imminent shutdown: replace placeholder in text
            let motd_text = motd.text.as_ref().map_or_else(
                || state.default_text().into_owned(),
                |text| text.replace("${seconds_remaining}", &seconds_remaining.to_string()),
            );
            generate_motd_packet_with_text(motd, &motd_text, false)
        }
        (_, Some(motd)) => generate_motd_packet(motd, use_default_favicon),
        // No custom config, use defaults
        (state, None) => {
            let default_motd = create_default_motd(state.default_text().into_owned());
            generate_motd_packet(&default_motd, use_default_favicon)
        }
    }
}

pub fn get_motd_config_for_state<'a>(
    state: &MotdState,
    server_motds: &'a infrarust_config::models::server::ServerMotds,
) -> Option<&'a MotdConfig> {
    match state {
        MotdState::Online => server_motds.online.as_ref(),
        MotdState::Offline => server_motds.offline.as_ref(),
        MotdState::Starting => server_motds.starting.as_ref(),
        MotdState::Stopping => server_motds.stopping.as_ref(),
        MotdState::ImminentShutdown { .. } => server_motds.shutting_down.as_ref(),
        MotdState::Crashed => server_motds.crashed.as_ref(),
        MotdState::Unreachable => server_motds.unreachable.as_ref(),
        MotdState::UnableToFetchStatus => server_motds.unable_status.as_ref(),
        MotdState::Unknown => server_motds.unknown.as_ref(),
        MotdState::UnknownServer => None,
    }
}

/// Extracts the JSON from the modern packet, parses it, and reformats
/// as a legacy 0xFF kick packet (Beta or 1.4+ format depending on variant).
pub fn generate_legacy_motd_from_packet(
    status_packet: &Packet,
    variant: &LegacyPingVariant,
) -> io::Result<Vec<u8>> {
    // Decode the modern status response to get the JSON
    let response: ClientBoundResponse = status_packet.decode().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to decode status packet: {}", e),
        )
    })?;

    let json: ResponseJSON = serde_json::from_str(&response.json_response.0).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse status JSON: {}", e),
        )
    })?;

    let motd = extract_motd_text(&json.description);

    Ok(build_legacy_kick_from_json(
        variant,
        &motd,
        json.players.online,
        json.players.max,
        json.version.protocol,
        &json.version.name,
    ))
}

pub fn generate_legacy_motd_for_state(
    state: &MotdState,
    motd_config: Option<&MotdConfig>,
    variant: &LegacyPingVariant,
) -> io::Result<Vec<u8>> {
    let default_text = state.default_text();
    let motd_text = motd_config
        .and_then(|c| c.text.as_deref())
        .unwrap_or_else(|| default_text.as_ref());

    let protocol = motd_config.and_then(|c| c.protocol_version).unwrap_or(0);

    let version_name = motd_config
        .and_then(|c| c.version_name.as_deref())
        .unwrap_or("Infrarust");

    let online = motd_config.and_then(|c| c.online_players).unwrap_or(0);

    let max = motd_config.and_then(|c| c.max_players).unwrap_or(0);

    Ok(build_legacy_kick_from_json(
        variant,
        motd_text,
        online,
        max,
        protocol,
        version_name,
    ))
}

fn build_legacy_kick_from_json(
    variant: &LegacyPingVariant,
    motd: &str,
    online: i32,
    max: i32,
    protocol: i32,
    version_name: &str,
) -> Vec<u8> {
    if variant.uses_v1_4_response_format() {
        build_legacy_kick_v1_4(protocol, version_name, motd, online, max)
    } else {
        build_legacy_kick_beta(motd, online, max)
    }
}

/// The description can be either a plain string `"text"` or a JSON object `{"text": "..."}`.
fn extract_motd_text(description: &serde_json::Value) -> String {
    match description {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(obj) => obj
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_for_state_offline() {
        let result = generate_for_state(&MotdState::Offline, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_for_state_with_custom_motd() {
        let custom_motd = MotdConfig {
            text: Some("Custom offline message".to_string()),
            version_name: Some("Test".to_string()),
            ..Default::default()
        };
        let result = generate_for_state(&MotdState::Offline, Some(&custom_motd));
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_for_state_imminent_shutdown() {
        let result = generate_for_state(
            &MotdState::ImminentShutdown {
                seconds_remaining: 30,
            },
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_for_state_imminent_shutdown_with_placeholder() {
        let custom_motd = MotdConfig {
            text: Some("Shutting down in ${seconds_remaining} seconds".to_string()),
            ..Default::default()
        };
        let result = generate_for_state(
            &MotdState::ImminentShutdown {
                seconds_remaining: 45,
            },
            Some(&custom_motd),
        );
        assert!(result.is_ok());
    }
}
