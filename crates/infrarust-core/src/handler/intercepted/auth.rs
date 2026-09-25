//! Authentication strategy for intercepted proxy modes.

use std::sync::{Arc, OnceLock};

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::lifecycle::{OnlineAuthFailed, PreLoginResult};
use infrarust_api::types::{GameProfile, PlayerId, ProfileProperty};
use infrarust_protocol::packets::login::{CLoginSuccess, Property, SLoginAcknowledged};
use infrarust_protocol::registry::{DecodedPacket, PacketRegistry};
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::auth::game_profile::offline_profile_uuid;
use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::types::LoginData;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

pub(super) struct Authenticated {
    pub profile: GameProfile,
    pub online_mode: bool,
}

pub(super) struct AuthResult {
    pub player_id: PlayerId,
    pub player_uuid: uuid::Uuid,
    pub username: String,
    pub api_profile: GameProfile,
    pub rewritten: bool,
}

impl AuthResult {
    pub(super) fn new(player_id: PlayerId, profile: GameProfile, rewritten: bool) -> Self {
        Self {
            player_id,
            player_uuid: profile.uuid,
            username: profile.username.clone(),
            api_profile: profile,
            rewritten,
        }
    }
}

pub(super) enum AuthStrategy {
    Mojang(Arc<MojangAuth>),
    Offline { mojang: Option<Arc<MojangAuth>> },
}

impl AuthStrategy {
    pub(super) const fn mode_label(&self) -> &'static str {
        match self {
            Self::Mojang(_) => "client_only",
            Self::Offline { .. } => "offline",
        }
    }

    pub(super) async fn authenticate(
        &self,
        client: &mut ClientBridge,
        login_data: Option<&LoginData>,
        services: &ProxyServices,
        version: ProtocolVersion,
        remote_addr: std::net::SocketAddr,
        domain: &str,
    ) -> Result<Authenticated, CoreError> {
        match self {
            Self::Mojang(auth) => {
                let login_data = login_data.ok_or(CoreError::MissingExtension("LoginData"))?;

                let pre_login_profile = GameProfile {
                    uuid: uuid::Uuid::nil(),
                    username: login_data.username.clone(),
                    properties: vec![],
                };
                let pre_login_result = fire_pre_login(
                    client,
                    pre_login_profile,
                    remote_addr,
                    version,
                    domain,
                    services,
                )
                .await?;

                if matches!(pre_login_result, PreLoginResult::ForceOffline) {
                    tracing::info!(
                        username = %login_data.username,
                        "ForceOffline: skipping Mojang auth for client_only player"
                    );
                    return Ok(Authenticated {
                        profile: offline_profile(
                            &login_data.username,
                            login_data.player_uuid,
                            services,
                        ),
                        online_mode: false,
                    });
                }

                online_auth(auth, client, login_data, services).await
            }
            Self::Offline { mojang } => {
                let username = login_data.map(|d| d.username.clone()).unwrap_or_default();
                let profile =
                    offline_profile(&username, login_data.and_then(|d| d.player_uuid), services);

                let pre_login_result = fire_pre_login(
                    client,
                    profile.clone(),
                    remote_addr,
                    version,
                    domain,
                    services,
                )
                .await?;

                if matches!(pre_login_result, PreLoginResult::ForceOnline) {
                    if let Some(auth) = mojang {
                        let login_data =
                            login_data.ok_or(CoreError::MissingExtension("LoginData"))?;
                        tracing::info!(
                            username = %login_data.username,
                            "ForceOnline: upgrading offline player to Mojang auth"
                        );
                        return online_auth(auth, client, login_data, services).await;
                    }
                    tracing::warn!(
                        "ForceOnline requested but no MojangAuth available, falling through to offline"
                    );
                }

                Ok(Authenticated {
                    profile,
                    online_mode: false,
                })
            }
        }
    }
}

fn offline_profile(
    username: &str,
    claimed: Option<uuid::Uuid>,
    services: &ProxyServices,
) -> GameProfile {
    GameProfile {
        uuid: offline_profile_uuid(services.config.auth.offline_uuid, username, claimed),
        username: username.to_string(),
        properties: vec![],
    }
}

async fn online_auth(
    auth: &MojangAuth,
    client: &mut ClientBridge,
    login_data: &LoginData,
    services: &ProxyServices,
) -> Result<Authenticated, CoreError> {
    let game_profile = match auth
        .authenticate(
            client,
            &login_data.username,
            login_data.profile_key.as_ref(),
            &services.packet_registry,
        )
        .await
    {
        Ok(profile) => profile,
        Err(e) => {
            tracing::warn!(
                username = %login_data.username,
                error = %e,
                "online authentication failed, the client will be disconnected"
            );
            let _ = services
                .event_bus
                .fire(OnlineAuthFailed {
                    username: login_data.username.clone(),
                })
                .await;
            return Err(e);
        }
    };

    tracing::info!(
        username = %game_profile.name,
        uuid = %game_profile.id,
        "client authenticated"
    );

    Ok(Authenticated {
        profile: GameProfile {
            uuid: game_profile.uuid().unwrap_or_else(|_| uuid::Uuid::new_v4()),
            username: game_profile.name.clone(),
            properties: game_profile
                .properties
                .iter()
                .map(|p| ProfileProperty {
                    name: p.name.clone(),
                    value: p.value.clone(),
                    signature: p.signature.clone(),
                })
                .collect(),
        },
        online_mode: true,
    })
}

pub(super) async fn complete_login(
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

/// Fires PreLoginEvent; returns `Err` if the player is denied, otherwise the result.
async fn fire_pre_login(
    client: &mut ClientBridge,
    profile: GameProfile,
    remote_addr: std::net::SocketAddr,
    version: ProtocolVersion,
    domain: &str,
    services: &ProxyServices,
) -> Result<PreLoginResult, CoreError> {
    let pre_login = infrarust_api::events::lifecycle::PreLoginEvent::new(
        profile,
        remote_addr,
        infrarust_api::types::ProtocolVersion::new(version.0),
        domain.to_string(),
    );
    let pre_login = services.event_bus.fire(pre_login).await;
    let result = pre_login.result().clone();
    if let PreLoginResult::Denied { reason } = &result {
        client
            .disconnect(reason, &services.packet_registry)
            .await
            .ok();
        return Err(CoreError::ConnectionClosed);
    }
    Ok(result)
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

/// Consumes LoginAcknowledged from client, transitions to Config state.
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
