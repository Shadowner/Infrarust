//! Authentication strategy for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::event::ResultedEvent;
use infrarust_api::types::PlayerId;
use infrarust_protocol::packets::login::{CLoginSuccess, Property, SLoginAcknowledged};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::types::LoginData;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

pub(super) struct AuthResult {
    pub player_id: PlayerId,
    pub player_uuid: uuid::Uuid,
    pub username: String,
    pub api_profile: infrarust_api::types::GameProfile,
    /// Whether LoginSuccess was sent to the client (true for ClientOnly).
    pub login_completed: bool,
}

pub(super) enum AuthStrategy {
    Mojang(Arc<MojangAuth>),
    None,
}

impl AuthStrategy {
    pub(super) const fn mode_label(&self) -> &'static str {
        match self {
            Self::Mojang(_) => "client_only",
            Self::None => "offline",
        }
    }

    /// Runs authentication + PreLogin/PostLogin events.
    ///
    /// `Mojang`: PreLoginEvent -> RSA exchange -> LoginSuccess -> LoginAcknowledged -> PostLoginEvent.
    /// `None`: PreLoginEvent -> PostLoginEvent (no packets exchanged).
    pub(super) async fn authenticate(
        &self,
        client: &mut ClientBridge,
        login_data: Option<&LoginData>,
        services: &ProxyServices,
        version: ProtocolVersion,
        peer_addr: std::net::SocketAddr,
        domain: &str,
    ) -> Result<AuthResult, CoreError> {
        match self {
            Self::Mojang(auth) => {
                let login_data =
                    login_data.ok_or_else(|| CoreError::MissingExtension("LoginData"))?;

                // nil UUID — real UUID comes from Mojang
                let pre_login_profile = infrarust_api::types::GameProfile {
                    uuid: uuid::Uuid::nil(),
                    username: login_data.username.clone(),
                    properties: vec![],
                };
                fire_pre_login(
                    client,
                    pre_login_profile,
                    peer_addr,
                    version,
                    domain,
                    services,
                )
                .await?;

                let game_profile = auth
                    .authenticate(client, &login_data.username, &services.packet_registry)
                    .await?;

                tracing::info!(
                    username = %game_profile.name,
                    uuid = %game_profile.id,
                    "client authenticated"
                );

                let player_uuid = game_profile.uuid().unwrap_or_else(|_| uuid::Uuid::new_v4());
                let player_id = crate::player::next_player_id();

                let login_props: Vec<Property> = game_profile
                    .properties
                    .iter()
                    .map(|p| Property {
                        name: p.name.clone(),
                        value: p.value.clone(),
                        signature: p.signature.clone(),
                    })
                    .collect();

                let api_profile = infrarust_api::types::GameProfile {
                    uuid: player_uuid,
                    username: game_profile.name.clone(),
                    properties: login_props
                        .iter()
                        .map(|p| infrarust_api::types::ProfileProperty {
                            name: p.name.clone(),
                            value: p.value.clone(),
                            signature: p.signature.clone(),
                        })
                        .collect(),
                };

                send_login_success(
                    client,
                    player_uuid,
                    &game_profile.name,
                    &login_props,
                    version,
                    &services.packet_registry,
                )
                .await?;

                services.event_bus.fire_and_forget_arc(
                    infrarust_api::events::lifecycle::PostLoginEvent {
                        profile: api_profile.clone(),
                        player_id,
                        protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
                    },
                );

                if version.no_less_than(ProtocolVersion::V1_20_2) {
                    consume_login_acknowledged(client, version, &services.packet_registry).await?;
                } else {
                    client.set_state(ConnectionState::Play);
                }

                Ok(AuthResult {
                    player_id,
                    player_uuid,
                    username: game_profile.name.clone(),
                    api_profile,
                    login_completed: true,
                })
            }
            Self::None => {
                let player_uuid = login_data
                    .and_then(|d| d.player_uuid)
                    .unwrap_or_else(uuid::Uuid::new_v4);
                let username = login_data.map(|d| d.username.clone()).unwrap_or_default();
                let player_id = crate::player::next_player_id();

                let api_profile = infrarust_api::types::GameProfile {
                    uuid: player_uuid,
                    username: username.clone(),
                    properties: vec![],
                };

                fire_pre_login(
                    client,
                    api_profile.clone(),
                    peer_addr,
                    version,
                    domain,
                    services,
                )
                .await?;

                services.event_bus.fire_and_forget_arc(
                    infrarust_api::events::lifecycle::PostLoginEvent {
                        profile: api_profile.clone(),
                        player_id,
                        protocol_version: infrarust_api::types::ProtocolVersion::new(version.0),
                    },
                );

                Ok(AuthResult {
                    player_id,
                    player_uuid,
                    username,
                    api_profile,
                    login_completed: false,
                })
            }
        }
    }
}

/// Fires PreLoginEvent; returns Err if the player is denied.
async fn fire_pre_login(
    client: &mut ClientBridge,
    profile: infrarust_api::types::GameProfile,
    peer_addr: std::net::SocketAddr,
    version: ProtocolVersion,
    domain: &str,
    services: &ProxyServices,
) -> Result<(), CoreError> {
    let pre_login = infrarust_api::events::lifecycle::PreLoginEvent::new(
        profile,
        peer_addr,
        infrarust_api::types::ProtocolVersion::new(version.0),
        domain.to_string(),
    );
    let pre_login = services.event_bus.fire(pre_login).await;
    if let infrarust_api::events::lifecycle::PreLoginResult::Denied { reason } = pre_login.result()
    {
        let reason_json = reason.to_json();
        client
            .disconnect(&reason_json, &services.packet_registry)
            .await
            .ok();
        return Err(CoreError::ConnectionClosed);
    }
    Ok(())
}

pub(super) async fn send_login_success(
    client: &mut ClientBridge,
    uuid: uuid::Uuid,
    username: &str,
    properties: &[Property],
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> Result<(), CoreError> {
    let login_success = CLoginSuccess {
        uuid,
        username: username.to_string(),
        properties: properties.to_vec(),
        strict_error_handling: version.no_less_than(ProtocolVersion::V1_20_5)
            && version.no_greater_than(ProtocolVersion::V1_21),
    };

    client.send_packet(&login_success, registry).await?;
    tracing::debug!("sent LoginSuccess to client");
    Ok(())
}

/// Consumes LoginAcknowledged from client, transitions to Config state.
pub(super) async fn consume_login_acknowledged(
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
