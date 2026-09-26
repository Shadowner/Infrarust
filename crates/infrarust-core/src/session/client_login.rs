use std::sync::OnceLock;

use infrarust_api::types::GameProfile;
use infrarust_protocol::packets::login::{CLoginSuccess, Property, SLoginAcknowledged};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::error::CoreError;
use crate::session::client_bridge::ClientBridge;

pub(crate) async fn complete_login(
    client: &mut ClientBridge,
    profile: &GameProfile,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
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
        consume_login_acknowledged(client, version, registry).await
    } else {
        client.set_state(ConnectionState::Play);
        Ok(())
    }
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
        _ => Err(CoreError::Auth(
            "expected LoginAcknowledged from client".to_string(),
        )),
    }
}
