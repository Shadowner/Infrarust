//! Shared test utilities for the limbo module.

#![cfg(test)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::{Bytes, BytesMut};
use infrarust_api::event::BoxFuture;
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

use infrarust_api::limbo::handler::{HandlerResult, LimboHandler};
use infrarust_api::limbo::session::LimboSession;
use infrarust_api::types::{GameProfile, PlayerId};
use infrarust_protocol::io::PacketFrame;
use infrarust_protocol::packets::Packet;
use infrarust_protocol::registry::PacketRegistry;
use infrarust_protocol::version::{ConnectionState, Direction, ProtocolVersion};

use crate::ban::manager::BanManager;
use crate::ban::storage::BanStorage;
use crate::ban::types::{BanEntry, BanTarget};
use crate::error::CoreError;
use crate::event_bus::bus::EventBusImpl;
use crate::filter::codec_registry::CodecFilterRegistryImpl;
use crate::filter::transport_chain::TransportFilterChain;
use crate::limbo::registry::LimboHandlerRegistry;
use crate::limbo::registry_cache::RegistryCodecCache;
use crate::player::registry::PlayerRegistryImpl;
use crate::registry::ConnectionRegistry;
use crate::routing::DomainRouter;
use crate::services::ProxyServices;
use crate::services::command_manager::CommandManagerImpl;
use crate::session::client_bridge::ClientBridge;

pub fn test_profile() -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::nil(),
        username: "LimboTester".to_string(),
        properties: vec![],
    }
}

pub fn test_registry() -> PacketRegistry {
    infrarust_protocol::registry::build_default_registry()
}

pub fn build_frame<P: Packet + 'static>(
    packet: &P,
    version: ProtocolVersion,
    registry: &PacketRegistry,
) -> PacketFrame {
    let packet_id = registry
        .get_packet_id::<P>(ConnectionState::Play, Direction::Serverbound, version)
        .expect("packet ID should exist in registry");
    let mut payload = Vec::new();
    packet.encode(&mut payload, version).unwrap();
    PacketFrame {
        id: packet_id,
        payload: Bytes::from(payload),
    }
}

/// Returns `(server-side ClientBridge, raw client TcpStream)` via loopback.
pub async fn test_client_bridge(version: ProtocolVersion) -> (ClientBridge, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client_stream = TcpStream::connect(addr).await.unwrap();
    let (server_stream, _) = listener.accept().await.unwrap();
    let mut bridge = ClientBridge::new(server_stream, BytesMut::new(), version);
    bridge.set_state(ConnectionState::Play);
    (bridge, client_stream)
}

pub fn test_proxy_services() -> ProxyServices {
    let connection_registry = Arc::new(ConnectionRegistry::new());
    let packet_registry = Arc::new(test_registry());
    let ban_storage: Arc<dyn BanStorage> = Arc::new(NullBanStorage);
    let provider: Arc<dyn crate::registry_data::RegistryDataProvider> =
        Arc::new(crate::registry_data::embedded::EmbeddedRegistryDataProvider);

    ProxyServices {
        event_bus: Arc::new(EventBusImpl::new()),
        player_registry: Arc::new(PlayerRegistryImpl::new(Arc::clone(&connection_registry))),
        command_manager: Arc::new(CommandManagerImpl::new()),
        connection_registry,
        packet_registry,
        server_manager: None,
        ban_manager: Arc::new(BanManager::new(
            ban_storage,
            Arc::new(ConnectionRegistry::new()),
        )),
        config: Arc::new(toml::from_str("").unwrap()),
        domain_router: Arc::new(DomainRouter::new()),
        codec_filter_registry: Arc::new(CodecFilterRegistryImpl::new()),
        transport_filter_chain: TransportFilterChain::empty(),
        limbo_handler_registry: Arc::new(LimboHandlerRegistry::new()),
        registry_codec_cache: Arc::new(RegistryCodecCache::new(provider)),
        provider_event_sender: tokio::sync::mpsc::channel(1).0,
        forwarding_mode: Arc::new(crate::forwarding::ForwardingMode::None),
        forwarding_secret: None,
        permission_service: Arc::new(crate::permissions::PermissionService::new_sync(
            &Default::default(),
        )),
    }
}

struct NullBanStorage;

impl BanStorage for NullBanStorage {
    fn add_ban(
        &self,
        _entry: BanEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn remove_ban(
        &self,
        _target: &BanTarget,
    ) -> Pin<Box<dyn Future<Output = Result<bool, CoreError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    fn is_banned(
        &self,
        _target: &BanTarget,
    ) -> Pin<Box<dyn Future<Output = Result<Option<BanEntry>, CoreError>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    fn check_player<'a>(
        &'a self,
        _ip: &'a IpAddr,
        _username: &'a str,
        _uuid: Option<&'a Uuid>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<BanEntry>, CoreError>> + Send + 'a>> {
        Box::pin(async { Ok(None) })
    }

    fn get_all_active(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<BanEntry>, CoreError>> + Send + '_>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn purge_expired(&self) -> Pin<Box<dyn Future<Output = Result<usize, CoreError>> + Send + '_>> {
        Box::pin(async { Ok(0) })
    }

    fn load(&self) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn save(&self) -> Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

pub struct FixedHandler {
    pub name: &'static str,
    pub result: HandlerResult,
}

impl LimboHandler for FixedHandler {
    fn name(&self) -> &str {
        self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

pub struct TrackingHandler {
    pub name: &'static str,
    pub result: HandlerResult,
    pub called: Arc<AtomicBool>,
}

impl LimboHandler for TrackingHandler {
    fn name(&self) -> &str {
        self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        self.called.store(true, Ordering::SeqCst);
        let result = self.result.clone();
        Box::pin(async move { result })
    }

    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

pub struct HoldHandler {
    pub name: &'static str,
}

impl LimboHandler for HoldHandler {
    fn name(&self) -> &str {
        self.name
    }

    fn on_player_enter<'a>(
        &'a self,
        _session: &'a dyn LimboSession,
    ) -> BoxFuture<'a, HandlerResult> {
        Box::pin(async { HandlerResult::Hold })
    }

    fn on_disconnect(&self, _player_id: PlayerId) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
