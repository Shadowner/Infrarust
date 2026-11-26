//! Authentication strategy for intercepted proxy modes.

use std::sync::Arc;

use infrarust_api::events::lifecycle::{OnlineAuthFailedEvent, PreLoginResult};
use infrarust_api::types::{Component, GameProfile, ProfileProperty};
use infrarust_protocol::version::ProtocolVersion;

use crate::auth::game_profile::offline_profile;
use crate::auth::mojang::MojangAuth;
use crate::error::CoreError;
use crate::pipeline::types::LoginData;
use crate::server::ProxyServices;
use crate::session::client_bridge::ClientBridge;
use crate::session::login_admission::{PreLogin, pre_login};

pub(crate) enum Authenticated {
    Denied(Component),
    Player {
        profile: GameProfile,
        online_mode: bool,
    },
}

enum Resumption<'a> {
    Plain,
    Encrypted(&'a MojangAuth),
    Declined,
}

pub(crate) enum AuthStrategy {
    Mojang(Arc<MojangAuth>),
    Offline { mojang: Option<Arc<MojangAuth>> },
}

impl AuthStrategy {
    pub(crate) const fn mode_label(&self) -> &'static str {
        match self {
            Self::Mojang(_) => "client_only",
            Self::Offline { .. } => "offline",
        }
    }

    pub(crate) async fn authenticate(
        &self,
        client: &mut ClientBridge,
        login_data: Option<&LoginData>,
        services: &ProxyServices,
        version: ProtocolVersion,
        remote_addr: std::net::SocketAddr,
        domain: &str,
    ) -> Result<Authenticated, CoreError> {
        let api_version = infrarust_api::types::ProtocolVersion::new(version.0);
        match self {
            Self::Mojang(auth) => {
                let login_data = login_data.ok_or(CoreError::MissingExtension("LoginData"))?;

                let pre_login_profile = GameProfile {
                    uuid: uuid::Uuid::nil(),
                    username: login_data.username.clone(),
                    properties: vec![],
                };
                let pre_login_result = match pre_login(
                    services,
                    pre_login_profile,
                    remote_addr,
                    api_version,
                    domain,
                )
                .await
                {
                    PreLogin::Denied(reason) => return Ok(Authenticated::Denied(reason)),
                    PreLogin::Proceed(result) => result,
                };

                if matches!(pre_login_result, PreLoginResult::ForceOffline) {
                    tracing::info!(
                        username = %login_data.username,
                        "ForceOffline: skipping Mojang auth for client_only player"
                    );
                    return Ok(Authenticated::Player {
                        profile: offline_profile(
                            services.config.auth.offline_uuid,
                            &login_data.username,
                            login_data.player_uuid,
                        )?,
                        online_mode: false,
                    });
                }

                online_auth(auth, client, login_data, services).await
            }
            Self::Offline { mojang } => {
                let username = login_data.map(|d| d.username.clone()).unwrap_or_default();
                let profile = offline_profile(
                    services.config.auth.offline_uuid,
                    &username,
                    login_data.and_then(|d| d.player_uuid),
                )?;

                let pre_login_result =
                    match pre_login(services, profile.clone(), remote_addr, api_version, domain)
                        .await
                    {
                        PreLogin::Denied(reason) => return Ok(Authenticated::Denied(reason)),
                        PreLogin::Proceed(result) => result,
                    };

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

                Ok(Authenticated::Player {
                    profile,
                    online_mode: false,
                })
            }
        }
    }

    pub(crate) async fn resume(
        &self,
        client: &mut ClientBridge,
        profile: GameProfile,
        online_mode: bool,
        services: &ProxyServices,
        remote_addr: std::net::SocketAddr,
        domain: &str,
    ) -> Result<Option<Authenticated>, CoreError> {
        let reauthenticate = services.config.restart.reauthenticate;
        let resumption = match (self, online_mode) {
            (Self::Mojang(auth) | Self::Offline { mojang: Some(auth) }, true)
                if !reauthenticate =>
            {
                Resumption::Encrypted(auth)
            }
            (Self::Offline { .. }, false) => Resumption::Plain,
            _ => Resumption::Declined,
        };
        if matches!(resumption, Resumption::Declined) {
            return Ok(None);
        }
        let version = infrarust_api::types::ProtocolVersion::new(client.protocol_version().0);
        match pre_login(services, profile.clone(), remote_addr, version, domain).await {
            PreLogin::Denied(reason) => return Ok(Some(Authenticated::Denied(reason))),
            PreLogin::Proceed(_) => {}
        }
        if let Resumption::Encrypted(auth) = resumption {
            auth.encrypt(client, &services.packet_registry).await?;
        }
        tracing::info!(
            username = %profile.username,
            uuid = %profile.uuid,
            online_mode,
            "player resumed after the restart"
        );
        Ok(Some(Authenticated::Player {
            profile,
            online_mode,
        }))
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
                .fire(OnlineAuthFailedEvent::new(login_data.username.clone()))
                .await;
            return Err(e);
        }
    };

    tracing::info!(
        username = %game_profile.name,
        uuid = %game_profile.id,
        "client authenticated"
    );

    Ok(Authenticated::Player {
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
