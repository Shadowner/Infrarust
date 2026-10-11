use std::sync::OnceLock;

use infrarust_protocol::VarInt;
use infrarust_protocol::packets::login::{
    CLoginSuccess, CSetCompression, Property, SLoginAcknowledged,
};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::event_bus::conversion::protocol_state_to_api;
use crate::session::client_bridge::ClientBridge;
use crate::session::context::{SessionContext, SessionIo};

pub(crate) async fn complete_login(
    ctx: &SessionContext<'_>,
    io: &mut SessionIo,
) -> Result<(), CoreError> {
    let version = ctx.version();
    let registry = ctx.registry();
    let client = &mut io.client;

    enable_compression(
        client,
        ctx.services.config.compression_threshold,
        version,
        registry,
    )
    .await?;

    let profile = ctx.profile();
    let properties: Vec<Property> = profile
        .properties
        .iter()
        .map(|p| Property {
            name: p.name.clone(),
            value: p.value.clone(),
            signature: p.signature.clone(),
        })
        .collect();
    send_login_success(
        client,
        profile.uuid,
        &profile.username,
        &properties,
        version,
        registry,
    )
    .await?;

    if version.no_less_than(ProtocolVersion::V1_20_2) {
        consume_login_acknowledged(client, version, registry).await?;
    } else {
        client.set_state(ConnectionState::Play);
    }

    if let Some(threshold) = io.client.compression_threshold() {
        io.client_codec.notify_compression_change(threshold);
        io.server_codec.notify_compression_change(threshold);
    }
    let state = protocol_state_to_api(io.client.state());
    io.client_codec.notify_state_change(state);
    io.server_codec.notify_state_change(state);
    Ok(())
}

async fn enable_compression(
    client: &mut ClientBridge,
    threshold: i32,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    if threshold < 0 || version.less_than(ProtocolVersion::V1_8) {
        return Ok(());
    }
    let packet = CSetCompression {
        threshold: VarInt(threshold),
    };
    client.send_packet(&packet, registry).await?;
    client.set_compression(threshold);
    tracing::debug!(threshold, "client compression activated");
    Ok(())
}

async fn send_login_success(
    client: &mut ClientBridge,
    uuid: uuid::Uuid,
    username: &str,
    properties: &[Property],
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    static SESSION_ID: OnceLock<uuid::Uuid> = OnceLock::new();

    let login_success = CLoginSuccess {
        uuid,
        username: username.to_string(),
        properties: properties.to_vec(),
        strict_error_handling: version.no_less_than(ProtocolVersion::V1_20_5)
            && version.no_greater_than(ProtocolVersion::V1_21),
        session_id: version
            .no_less_than(ProtocolVersion::V26_2)
            .then(|| *SESSION_ID.get_or_init(uuid::Uuid::new_v4)),
    };

    client.send_packet(&login_success, registry).await?;
    tracing::debug!("sent LoginSuccess to client");
    Ok(())
}

async fn consume_login_acknowledged(
    client: &mut ClientBridge,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let frame = client
        .read_frame()
        .await?
        .ok_or(CoreError::ConnectionClosed)?;

    let decoded = registry.decode_frame(
        &frame,
        ConnectionState::Login,
        Direction::Serverbound,
        version,
    )?;

    match decoded {
        DecodedPacket::Typed { packet, .. }
            if packet
                .as_any()
                .downcast_ref::<SLoginAcknowledged>()
                .is_some() =>
        {
            client.set_state(ConnectionState::Config);
            tracing::debug!("client LoginAcknowledged -> Config");
            Ok(())
        }
        _ => Err(CoreError::Protocol(
            infrarust_protocol::ProtocolError::invalid("expected LoginAcknowledged from client"),
        )),
    }
}
