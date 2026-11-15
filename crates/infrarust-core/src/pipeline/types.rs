use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use infrarust_config::{ServerAddress, ServerConfig};
use infrarust_protocol::ProtocolVersion;
use infrarust_protocol::legacy::{LegacyHandshakeRequest, LegacyPingRequest};
use infrarust_protocol::packets::ProfileKey;
use smallvec::SmallVec;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionIntent {
    Status,
    Login,
    Transfer,
}

#[derive(Debug, Clone)]
pub struct HandshakeData {
    pub domain: String,
    pub raw_host: String,
    pub port: u16,
    pub protocol_version: ProtocolVersion,
    pub intent: ConnectionIntent,
    pub raw_packets: Vec<BytesMut>,
}

impl HandshakeData {
    pub fn host_suffix(&self) -> &str {
        crate::util::host_suffix(&self.raw_host)
    }
}

#[derive(Clone)]
pub struct RoutingData {
    pub server_config: Arc<ServerConfig>,
    pub config_id: String,
    pub load_balancer: Arc<dyn crate::loadbalancer::LoadBalancer>,
}

impl std::fmt::Debug for RoutingData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingData")
            .field("server_config", &self.server_config)
            .field("config_id", &self.config_id)
            .field("load_balancer", &self.load_balancer.name())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct LoginData {
    pub username: String,
    pub player_uuid: Option<Uuid>,
    pub profile_key: Option<ProfileKey>,
    pub raw_payload: Bytes,
    pub layout: LoginStartLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStartLayout {
    Declared,
    Mismatched,
}

#[derive(Debug, Clone)]
pub enum LegacyHello {
    Ping {
        request: LegacyPingRequest,
        raw: Vec<u8>,
    },
    Login {
        request: LegacyHandshakeRequest,
        raw: Vec<u8>,
    },
}

impl LegacyHello {
    pub const fn is_hostless(&self) -> bool {
        matches!(self, Self::Ping { request, .. } if request.hostname.is_none())
    }
}

#[derive(Debug, Clone)]
pub struct BackendTargets {
    pub addresses: SmallVec<[ServerAddress; 4]>,
}

impl BackendTargets {
    pub fn addresses_or_config(
        targets: Option<&Self>,
        server: &ServerConfig,
    ) -> Vec<ServerAddress> {
        targets.map_or_else(|| server.address_list(), |t| t.addresses.to_vec())
    }
}
