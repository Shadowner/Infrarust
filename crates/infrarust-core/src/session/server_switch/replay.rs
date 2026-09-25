use bytes::Bytes;
use infrarust_api::messaging::ChannelId;
use infrarust_api::types::ProtocolVersion as ApiVersion;
use infrarust_config::ServerConfig;
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::packets::config::SConfigClientInformation;
use infrarust_protocol::packets::play::client_information::{
    ClientInformation, SClientInformation,
};
use infrarust_protocol::version::{ConnectionState, ProtocolVersion};

use crate::error::CoreError;
use crate::player::PlayerSession;
use crate::plugin_messaging::channels::{self, MessageIds};
use crate::services::ProxyServices;
use crate::session::backend_bridge::BackendBridge;

fn information_frame(
    id: i32,
    information: ClientInformation,
    state: ConnectionState,
    version: ProtocolVersion,
) -> Option<PacketFrame> {
    let mut payload = Vec::new();
    let encoded = match state {
        ConnectionState::Config => {
            SConfigClientInformation { information }.encode(&mut payload, version)
        }
        _ => SClientInformation { information }.encode(&mut payload, version),
    };
    match encoded {
        Ok(()) => Some(PacketFrame::new(id, Bytes::from(payload))),
        Err(e) => {
            tracing::warn!("could not replay the client information: {e}");
            None
        }
    }
}

pub(super) fn client_state_frames(
    session: &PlayerSession,
    services: &ProxyServices,
    target: &ServerConfig,
    state: ConnectionState,
    version: ProtocolVersion,
) -> Vec<PacketFrame> {
    let ids = MessageIds::resolve(&services.packet_registry, version);
    let api_version = ApiVersion::new(version.0);
    let client = session.client_state();

    let information = client
        .information()
        .zip(ids.information(state))
        .and_then(|(information, id)| information_frame(id, information, state, version));

    let Some(message_id) = ids.serverbound(state) else {
        return information.into_iter().collect();
    };

    let brand = client.brand().map(|brand| {
        channels::build(
            message_id,
            ChannelId::brand().wire_name(api_version),
            &channels::brand_payload(&brand, version),
            version,
        )
    });

    let mut names = client.channels();
    for announced in services
        .plugin_messaging
        .announced_channels(Some(target), version)
    {
        if !names.contains(&announced) {
            names.push(announced);
        }
    }
    let register = ChannelId::register();
    let register = register.wire_name(api_version);
    let registrations = channels::channel_payloads(names.iter().map(String::as_str))
        .into_iter()
        .map(|payload| channels::build(message_id, register, &payload, version));

    let mut frames = Vec::new();
    if state == ConnectionState::Config {
        frames.extend(brand);
        frames.extend(information);
    } else {
        frames.extend(information);
        frames.extend(brand);
    }
    frames.extend(registrations);
    frames
}

pub(super) async fn replay(
    backend: &mut BackendBridge,
    session: &PlayerSession,
    services: &ProxyServices,
    target: &ServerConfig,
    version: ProtocolVersion,
) -> Result<(), CoreError> {
    let frames = client_state_frames(session, services, target, backend.state, version);
    if frames.is_empty() {
        return Ok(());
    }
    for frame in &frames {
        backend.queue_frame(frame)?;
    }
    backend.flush().await
}
