#![allow(clippy::unwrap_used, clippy::expect_used)]
mod capabilities;
mod claims;
mod codec;
mod commands;
mod config;
mod event_bus;
mod limbo;
mod load_balancer;
mod log;
mod messaging;
mod permission_nodes;
mod ping;
mod players;
mod proxy_info;
mod scheduler;
mod text;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use bytes::Bytes;
use infrarust_api::error::PlayerError;
use infrarust_api::event::BoxFuture;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::messaging::ChannelId;
use infrarust_api::permissions::{Capability, CapabilitySet};
use infrarust_api::player::{
    BossBar, BossBarControl, BossBarHandle, BossBarUpdate, ClientSettings, ConnectionResult,
    Player, ResourcePackRequest,
};
use infrarust_api::plugin::PluginContext;
use infrarust_api::test_util::{
    MockConfigService, MockLoadBalancerService, MockPlayer, MockPlayerRegistry,
};
use infrarust_api::types::{
    Component, GameProfile, PlayerId, ProtocolVersion, RawPacket, ServerAddress, ServerId,
    TitleData,
};
use infrarust_config::WasmQuotasConfig;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions};

use crate::bindings::infrarust::plugin::{ban_service, command_manager, types as wt};
use crate::component;
use crate::config::SandboxLimits;
use crate::store_state::{PluginStoreState, build_probe_state};

fn context(players: Vec<Arc<dyn Player>>) -> Arc<dyn PluginContext> {
    PluginContextFactoryImpl::new(services(players), HashMap::new()).create_context("test")
}

fn services(players: Vec<Arc<dyn Player>>) -> PluginServices {
    let registry = MockPlayerRegistry::new();
    for player in players {
        registry.add_dyn(player);
    }
    let lobby_backend = ServerAddress {
        host: "10.0.0.2".to_owned(),
        port: 25565,
    };
    PluginServices {
        player_registry: Arc::new(registry),
        config_service: Arc::new(MockConfigService::new().with_value("greeting", "hello")),
        load_balancer_service: Arc::new(MockLoadBalancerService::new().with_server(
            "lobby",
            "round_robin",
            [lobby_backend],
        )),
        ..PluginServices::for_tests()
    }
}

fn state_with(capabilities: CapabilitySet, players: Vec<Arc<dyn Player>>) -> PluginStoreState {
    build_probe_state("test".to_owned(), &SandboxLimits::default())
        .with_capabilities(capabilities)
        .with_ctx(context(players))
}

fn text_of(message: &str) -> wt::Component {
    component::to_wit(&Component::text(message))
}

fn stalled_player() -> Arc<dyn Player> {
    MockPlayer::new(1, "tester")
        .online_mode(true)
        .stalled()
        .into_arc()
}

fn nobody() -> ban_service::BanTarget {
    ban_service::BanTarget::Username("nobody".to_owned())
}

fn is_denied(result: &Result<impl std::fmt::Debug, wt::HostError>, capability: Capability) -> bool {
    matches!(result, Err(e) if e.kind == wt::ErrorKind::PermissionDenied
        && e.message == format!("missing capability: {}", capability.to_kebab()))
}

fn address() -> wt::ServerAddress {
    wt::ServerAddress {
        host: "10.0.0.2".into(),
        port: 25565,
    }
}

fn channel() -> wt::ChannelId {
    wt::ChannelId {
        modern: Some("test:echo".into()),
        legacy: None,
    }
}

fn command_spec() -> command_manager::CommandSpec {
    command_manager::CommandSpec {
        name: "probe".to_owned(),
        aliases: vec![],
        description: String::new(),
        usage: None,
        permission: None,
        hidden: false,
    }
}

#[derive(Default)]
struct Recorded {
    messages: Mutex<Vec<(String, Vec<u8>, bool)>>,
    bars: Mutex<Vec<(uuid::Uuid, Option<BossBarUpdate>)>>,
    cookies: Mutex<HashMap<String, Bytes>>,
    packs: Mutex<Vec<ResourcePackRequest>>,
}

impl BossBarControl for Recorded {
    fn update(&self, id: uuid::Uuid, update: BossBarUpdate) -> Result<(), PlayerError> {
        self.bars.lock().unwrap().push((id, Some(update)));
        Ok(())
    }

    fn hide(&self, id: uuid::Uuid) -> Result<(), PlayerError> {
        self.bars.lock().unwrap().push((id, None));
        Ok(())
    }
}

struct RecordingPlayer {
    profile: GameProfile,
    seen: Arc<Recorded>,
}

impl RecordingPlayer {
    fn shared() -> (Arc<dyn Player>, Arc<Recorded>) {
        let seen = Arc::new(Recorded::default());
        let player = Arc::new(Self {
            profile: GameProfile {
                uuid: uuid::Uuid::from_u128(1),
                username: "Steve".to_owned(),
                properties: vec![],
            },
            seen: Arc::clone(&seen),
        });
        (player, seen)
    }
}

impl infrarust_api::player::private::Sealed for RecordingPlayer {}

impl Player for RecordingPlayer {
    fn id(&self) -> PlayerId {
        PlayerId::new(1)
    }
    fn profile(&self) -> &GameProfile {
        &self.profile
    }
    fn protocol_version(&self) -> ProtocolVersion {
        ProtocolVersion::MINECRAFT_1_21
    }
    fn remote_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }
    fn current_server(&self) -> Option<ServerId> {
        Some(ServerId::new("lobby"))
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn is_active(&self) -> bool {
        true
    }
    fn disconnect(&self, _reason: Component) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn send_message(&self, _message: Component) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_title(&self, _title: TitleData) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_action_bar(&self, _message: Component) -> Result<(), PlayerError> {
        Ok(())
    }
    fn send_packet(&self, _packet: RawPacket) -> Result<(), PlayerError> {
        Ok(())
    }
    fn switch_server(&self, _target: ServerId) -> BoxFuture<'_, Result<(), PlayerError>> {
        Box::pin(async { Ok(()) })
    }
    fn is_online_mode(&self) -> bool {
        true
    }
    fn has_permission(&self, _permission: &str) -> bool {
        false
    }
    fn refresh_permissions(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn connected_at(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
    fn settings(&self) -> Option<ClientSettings> {
        Some(ClientSettings::new("fr_fr"))
    }
    fn known_channels(&self) -> Vec<String> {
        vec!["test:echo".to_owned()]
    }
    fn send_plugin_message(&self, channel: &ChannelId, data: Bytes) -> Result<(), PlayerError> {
        let entry = (channel.to_string(), data.to_vec(), false);
        self.seen.messages.lock().unwrap().push(entry);
        Ok(())
    }
    fn send_plugin_message_to_backend(
        &self,
        channel: &ChannelId,
        data: Bytes,
    ) -> Result<(), PlayerError> {
        let entry = (channel.to_string(), data.to_vec(), true);
        self.seen.messages.lock().unwrap().push(entry);
        Ok(())
    }
    fn connect(&self, target: ServerId) -> BoxFuture<'_, Result<ConnectionResult, PlayerError>> {
        Box::pin(async move {
            Ok(if target.as_str() == "lobby" {
                ConnectionResult::AlreadyConnected
            } else {
                ConnectionResult::Denied(Component::text("full"))
            })
        })
    }
    fn show_boss_bar(&self, _bar: BossBar) -> Result<BossBarHandle, PlayerError> {
        let control: Arc<dyn BossBarControl> = Arc::clone(&self.seen) as Arc<dyn BossBarControl>;
        Ok(BossBarHandle::new(uuid::Uuid::from_u128(42), control))
    }
    fn send_resource_pack(&self, pack: ResourcePackRequest) -> Result<(), PlayerError> {
        self.seen.packs.lock().unwrap().push(pack);
        Ok(())
    }
    fn store_cookie(&self, key: &str, data: Bytes) -> Result<(), PlayerError> {
        self.seen
            .cookies
            .lock()
            .unwrap()
            .insert(key.to_owned(), data);
        Ok(())
    }
    fn request_cookie(&self, key: &str) -> BoxFuture<'_, Result<Option<Bytes>, PlayerError>> {
        let cookie = self.seen.cookies.lock().unwrap().get(key).cloned();
        Box::pin(async move { Ok(cookie) })
    }
}

fn quota_state(quotas: WasmQuotasConfig, grants: &[Capability]) -> PluginStoreState {
    let permissions = PluginPermissions {
        permissions: grants.iter().map(|c| c.to_kebab().to_owned()).collect(),
        ..PluginPermissions::default()
    };
    let factory = PluginContextFactoryImpl::new(
        services(vec![]),
        HashMap::from([("test".to_owned(), permissions)]),
    );
    let capabilities = grants
        .iter()
        .fold(CapabilitySet::baseline(), |set, capability| {
            set.with(*capability)
        });
    let sandbox = SandboxLimits {
        quotas,
        ..SandboxLimits::default()
    };
    build_probe_state("test".to_owned(), &sandbox)
        .with_capabilities(capabilities)
        .with_ctx(factory.create_context("test"))
}

fn quotas(adjust: impl FnOnce(&mut WasmQuotasConfig)) -> WasmQuotasConfig {
    let mut quotas = WasmQuotasConfig::default();
    adjust(&mut quotas);
    quotas
}

fn is_limit_exceeded<T: std::fmt::Debug>(result: &Result<T, wt::HostError>, key: &str) -> bool {
    matches!(result, Err(e) if e.kind == wt::ErrorKind::LimitExceeded
        && e.message.ends_with(&format!("(quotas.{key})")))
}
