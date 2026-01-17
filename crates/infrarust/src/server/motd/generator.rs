use infrarust_config::models::server::MotdConfig;
use infrarust_protocol::{
    minecraft::java::status::clientbound_response::{
        CLIENTBOUND_RESPONSE_ID, ClientBoundResponse, PlayersJSON, ResponseJSON, VersionJSON,
    },
    types::ProtocolString,
};

use crate::network::{
    packet::{Packet, PacketCodec},
    proxy_protocol::errors::ProxyProtocolError,
};

#[cfg(feature = "telemetry")]
use crate::telemetry::TELEMETRY;

use super::{favicon::{get_default_favicon, parse_favicon}, state::MotdState};

pub fn generate_motd_packet(
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
                || state.default_text(),
                |text| text.replace("${seconds_remaining}", &seconds_remaining.to_string()),
            );

            let mut motd = motd.clone();
            motd.text = Some(motd_text);
            generate_motd_packet(&motd, false)
        }
        (_, Some(motd)) => generate_motd_packet(motd, use_default_favicon),
        // No custom config, use defaults
        (state, None) => {
            let default_motd = create_default_motd(state.default_text());
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
