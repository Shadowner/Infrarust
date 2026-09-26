use std::net::IpAddr;
use std::sync::Arc;

use infrarust_api::player::Player;
use infrarust_api::types::{GameProfile, PlayerId, ServerId};
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::ProtocolVersion;
use infrarust_transport::{BackendConnector, ConnectionInfo};
use tokio_util::sync::CancellationToken;

use crate::filter::codec_chain::CodecFilterChain;
use crate::forwarding::ForwardingData;
use crate::pipeline::types::HandshakeData;
use crate::player::PlayerSession;
use crate::player::commands::CommandInbox;
use crate::plugin_messaging::router::Scope;
use crate::services::ProxyServices;
use crate::session::client_bridge::ClientBridge;

pub(crate) struct SessionContext<'a> {
    pub(crate) services: &'a ProxyServices,
    pub(crate) backend_connector: &'a BackendConnector,
    pub(crate) session: Arc<PlayerSession>,
    pub(crate) handshake: HandshakeData,
    pub(crate) connection_info: ConnectionInfo,
    pub(crate) token: CancellationToken,
}

impl SessionContext<'_> {
    pub(crate) const fn version(&self) -> ProtocolVersion {
        self.handshake.protocol_version
    }

    pub(crate) fn registry(&self) -> &PacketRegistry {
        &self.services.packet_registry
    }

    pub(crate) fn player_id(&self) -> PlayerId {
        self.session.id()
    }

    pub(crate) fn profile(&self) -> &GameProfile {
        self.session.game_profile()
    }

    pub(crate) fn username(&self) -> &str {
        &self.profile().username
    }

    pub(crate) fn player(&self) -> Arc<dyn Player> {
        Arc::clone(&self.session) as Arc<dyn Player>
    }

    pub(crate) fn real_ip(&self) -> IpAddr {
        self.connection_info
            .real_ip
            .unwrap_or(self.connection_info.peer_addr.ip())
    }

    pub(crate) fn forwarding_data(&self) -> ForwardingData {
        let profile = self.profile();
        ForwardingData {
            real_ip: self.real_ip(),
            uuid: profile.uuid,
            username: profile.username.clone(),
            properties: profile.properties.clone(),
            protocol_version: self.version(),
            chat_session: None,
        }
    }

    pub(crate) fn scope<'s>(&'s self, server: &'s ServerId) -> Scope<'s> {
        Scope {
            services: self.services,
            session: &self.session,
            server,
            version: self.version(),
        }
    }
}

pub(crate) struct SessionIo {
    pub(crate) client: ClientBridge,
    pub(crate) commands: CommandInbox,
    pub(crate) client_codec: CodecFilterChain,
    pub(crate) server_codec: CodecFilterChain,
}

impl SessionIo {
    pub(crate) fn close(&mut self) {
        self.client_codec.close();
        self.server_codec.close();
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::filter::codec_chain::build_codec_chains;
    use crate::pipeline::types::ConnectionIntent;

    pub(crate) fn test_context<'a>(
        services: &'a ProxyServices,
        connector: &'a BackendConnector,
        session: Arc<PlayerSession>,
    ) -> SessionContext<'a> {
        let peer_addr = "127.0.0.1:40000".parse().unwrap();
        SessionContext {
            services,
            backend_connector: connector,
            session,
            handshake: HandshakeData {
                domain: "origin.test".to_string(),
                raw_host: "origin.test".to_string(),
                port: 25565,
                protocol_version: ProtocolVersion::V1_21,
                intent: ConnectionIntent::Login,
                raw_packets: vec![],
            },
            connection_info: ConnectionInfo {
                peer_addr,
                real_ip: None,
                real_port: None,
                local_addr: peer_addr,
                connected_at: tokio::time::Instant::now(),
            },
            token: CancellationToken::new(),
        }
    }

    pub(crate) fn test_connector() -> BackendConnector {
        BackendConnector::new(
            std::time::Duration::from_secs(2),
            infrarust_config::KeepaliveConfig::default(),
        )
    }

    pub(crate) fn test_io(ctx: &SessionContext<'_>, client: ClientBridge) -> SessionIo {
        let (client_codec, server_codec) = build_codec_chains(
            &ctx.services.codec_filter_registry,
            infrarust_api::types::ProtocolVersion::new(ctx.version().0),
            ctx.player_id().as_u64(),
            ctx.connection_info.peer_addr,
            None,
        );
        SessionIo {
            client,
            commands: CommandInbox::new(PlayerSession::channel().1),
            client_codec,
            server_codec,
        }
    }
}
