use std::time::{Duration, SystemTime, UNIX_EPOCH};

use infrarust_api::player::Player;
use infrarust_api::types::ServerId;
use infrarust_config::ProxyMode;

use super::state::{ProfileRecord, SessionMeta};
use super::unix_millis;
use crate::player::PlayerSession;

pub(crate) fn capture(session: &PlayerSession, server: &ServerId, mode: ProxyMode) -> SessionMeta {
    SessionMeta {
        player_id: session.id().as_u64(),
        profile: ProfileRecord::from(session.profile()),
        protocol_version: session.protocol_version().raw(),
        mode,
        client_addr: session.remote_addr(),
        server: server.as_str().to_owned(),
        connected_address: session
            .connected_address()
            .map(|address| address.to_string()),
        virtual_host: session.virtual_host(),
        connected_at_ms: unix_millis(session.connected_at()),
    }
}

pub fn connected_at(meta: &SessionMeta) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(meta.connected_at_ms)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use infrarust_api::types::{GameProfile, PlayerId, ProfileProperty, ProtocolVersion};
    use infrarust_config::ServerAddress;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::loadbalancer::BackendLoad;
    use crate::player::SessionKind;

    #[test]
    fn capture_records_what_the_new_image_needs_to_rebuild_the_player() {
        let (tx, _rx) = PlayerSession::channel();
        let connected = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        let session = PlayerSession::builder(
            PlayerId::new(42),
            GameProfile {
                uuid: uuid::Uuid::from_u128(7),
                username: "Steve".to_string(),
                properties: vec![ProfileProperty {
                    name: "textures".to_string(),
                    value: "abc".to_string(),
                    signature: Some("sig".to_string()),
                }],
            },
            ProtocolVersion::new(767),
            "203.0.113.9:40000".parse().unwrap(),
            tx,
            CancellationToken::new(),
            Arc::new(BackendLoad::new()),
        )
        .kind(SessionKind::Forwarded)
        .virtual_host("lobby.test")
        .connected_at(connected)
        .build();
        session.set_connected_address(Some(ServerAddress {
            host: "10.0.0.5".to_string(),
            port: 25566,
        }));

        let meta = capture(&session, &ServerId::new("lobby"), ProxyMode::ZeroCopy);

        assert_eq!(meta.player_id, 42);
        assert_eq!(meta.profile.username, "Steve");
        assert_eq!(meta.profile.properties.len(), 1);
        assert_eq!(meta.protocol_version, 767);
        assert_eq!(meta.mode, ProxyMode::ZeroCopy);
        assert_eq!(meta.client_addr, "203.0.113.9:40000".parse().unwrap());
        assert_eq!(meta.server, "lobby");
        assert_eq!(meta.connected_address.as_deref(), Some("10.0.0.5:25566"));
        assert_eq!(meta.virtual_host.as_deref(), Some("lobby.test"));
        assert_eq!(meta.connected_at_ms, 1_700_000_000_123);
        assert_eq!(connected_at(&meta), connected);
        assert_eq!(GameProfile::from(meta.profile), session.profile().clone());
    }
}
