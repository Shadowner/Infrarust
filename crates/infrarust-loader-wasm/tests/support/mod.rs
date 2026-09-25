#![allow(dead_code)]

pub mod conformance;
pub mod mock_services;
pub mod native_scripted;
#[path = "../fixtures/scripted/src/script.rs"]
pub mod script;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use infrarust_api::command::CommandManager;
use infrarust_api::services::ban_service::BanService;
use infrarust_api::services::config_service::ConfigService;
use infrarust_api::services::player_registry::PlayerRegistry;
use infrarust_api::services::proxy_info::ProxyInfo;
use infrarust_api::types::GameProfile;
use infrarust_core::event_bus::{EventBusConfig, EventBusImpl};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::filter::transport_registry::TransportFilterRegistryImpl;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions, PluginRegistryImpl};
use infrarust_core::routing::DomainRouter;
use infrarust_core::services::command_manager::CommandManagerImpl;
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_core::services::server_manager_bridge::NoopServerManager;
use tokio_util::sync::CancellationToken;

use mock_services::{
    MockBanService, MockConfigService, MockLoadBalancerService, MockPlayerRegistry,
};

pub struct TestEnv {
    pub factory: PluginContextFactoryImpl,
    pub event_bus: Arc<EventBusImpl>,
    pub command_manager: Arc<CommandManagerImpl>,
    pub codec_registry: Arc<CodecFilterRegistryImpl>,
}

pub struct EnvOptions {
    pub player_registry: Arc<dyn PlayerRegistry>,
    pub config_service: Arc<dyn ConfigService>,
    pub ban_service: Arc<dyn BanService>,
    pub bus_config: EventBusConfig,
    pub grants: HashMap<String, PluginPermissions>,
}

impl Default for EnvOptions {
    fn default() -> Self {
        Self {
            player_registry: Arc::new(MockPlayerRegistry),
            config_service: Arc::new(MockConfigService),
            ban_service: Arc::new(MockBanService),
            bus_config: EventBusConfig::default(),
            grants: HashMap::new(),
        }
    }
}

impl EnvOptions {
    pub fn grant(mut self, plugin_id: &str, permission: &str) -> Self {
        self.grants
            .entry(plugin_id.to_owned())
            .or_default()
            .permissions
            .push(permission.to_owned());
        self
    }

    pub fn deny(mut self, plugin_id: &str, capability: &str) -> Self {
        self.grants
            .entry(plugin_id.to_owned())
            .or_default()
            .deny
            .push(capability.to_owned());
        self
    }
}

pub fn make_env(plugins_dir: PathBuf) -> TestEnv {
    make_env_with(plugins_dir, EnvOptions::default())
}

pub fn make_env_with(plugins_dir: PathBuf, options: EnvOptions) -> TestEnv {
    let event_bus = Arc::new(EventBusImpl::with_config(options.bus_config));
    let command_manager = Arc::new(CommandManagerImpl::new());
    let codec_registry = Arc::new(CodecFilterRegistryImpl::new());
    let services = PluginServices {
        event_bus: Arc::clone(&event_bus),
        player_registry: options.player_registry,
        server_manager: Arc::new(NoopServerManager),
        ban_service: options.ban_service,
        command_manager: Arc::clone(&command_manager) as Arc<dyn CommandManager>,
        scheduler: Arc::new(SchedulerImpl::new()),
        config_service: options.config_service,
        load_balancer_service: Arc::new(MockLoadBalancerService),
        plugin_registry: Arc::new(PluginRegistryImpl::new()),
        codec_filter_registry: Arc::clone(&codec_registry),
        transport_filter_registry: Arc::new(TransportFilterRegistryImpl::new()),
        domain_router: Arc::new(DomainRouter::new()),
        proxy_shutdown: CancellationToken::new(),
        proxy_info: ProxyInfo::default(),
        plugins_dir,
    };
    TestEnv {
        factory: PluginContextFactoryImpl::new(services, options.grants),
        event_bus,
        command_manager,
        codec_registry,
    }
}

pub fn session_player(
    id: u64,
    profile: GameProfile,
    protocol: i32,
    remote_addr: std::net::SocketAddr,
) -> Arc<dyn infrarust_api::player::Player> {
    let (commands, _) = infrarust_core::player::PlayerSession::channel();
    Arc::new(infrarust_core::player::PlayerSession::new(
        infrarust_api::types::PlayerId::new(id),
        profile,
        infrarust_api::types::ProtocolVersion::new(protocol),
        remote_addr,
        None,
        true,
        true,
        commands,
        CancellationToken::new(),
        infrarust_core::permissions::default_checker(),
        Arc::new(infrarust_core::loadbalancer::BackendLoad::new()),
    ))
}

pub fn nil_profile(username: &str) -> GameProfile {
    GameProfile {
        uuid: uuid::Uuid::nil(),
        username: username.to_owned(),
        properties: vec![],
    }
}

pub fn read_log(data_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(data_dir.join(script::LOG_FILE))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

pub fn write_script(plugins_dir: &Path, plugin_id: &str, script: &str) {
    let data_dir = plugins_dir.join(plugin_id);
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join(script::SCRIPT_FILE), script).unwrap();
}

#[cfg(feature = "wasm")]
pub fn fresh_loader() -> infrarust_loader_wasm::WasmPluginLoader {
    loader_from_toml("")
}

#[cfg(feature = "wasm")]
pub fn loader_from_toml(proxy_toml: &str) -> infrarust_loader_wasm::WasmPluginLoader {
    let config: infrarust_config::ProxyConfig = toml::from_str(proxy_toml).expect("proxy config");
    infrarust_loader_wasm::WasmPluginLoader::new(
        infrarust_loader_wasm::build_engine(&config).expect("build engine"),
        infrarust_loader_wasm::WasmLoaderConfig::from_proxy_config(&config),
    )
}

#[cfg(feature = "wasm")]
pub async fn load_enabled(
    loader: &infrarust_loader_wasm::WasmPluginLoader,
    factory: &PluginContextFactoryImpl,
    id: &str,
) -> Box<dyn infrarust_api::plugin::Plugin> {
    use infrarust_api::loader::{PluginContextFactory, PluginLoader};

    let plugin = loader
        .load(id, factory)
        .await
        .unwrap_or_else(|e| panic!("load {id}: {e}"));
    let ctx = factory.create_context(id);
    plugin
        .on_enable(ctx.as_ref())
        .await
        .unwrap_or_else(|e| panic!("enable {id}: {e}"));
    plugin
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("INFRARUST_WASM_FIXTURE_DIR"))
        .join(format!("fixture_{}.wasm", name.replace('-', "_")))
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub fn add_fixture(plugins_dir: &Path, fixture: &str, file_stem: &str) {
    std::fs::copy(
        fixture_path(fixture),
        plugins_dir.join(format!("{file_stem}.wasm")),
    )
    .unwrap_or_else(|e| panic!("staging fixture {fixture}: {e}"));
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub fn stage(fixture: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let plugins_dir = tmp.path().to_path_buf();
    add_fixture(&plugins_dir, fixture, fixture);
    (tmp, plugins_dir)
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub async fn add_precompiled_fixture(plugins_dir: &Path, fixture: &str) {
    use infrarust_api::loader::PluginLoader;

    type Artifacts = Vec<(std::ffi::OsString, Vec<u8>)>;
    static COMPILED: tokio::sync::Mutex<std::collections::BTreeMap<String, Artifacts>> =
        tokio::sync::Mutex::const_new(std::collections::BTreeMap::new());

    add_fixture(plugins_dir, fixture, fixture);
    let mut compiled = COMPILED.lock().await;
    if !compiled.contains_key(fixture) {
        let tmp = tempfile::tempdir().unwrap();
        add_fixture(tmp.path(), fixture, fixture);
        fresh_loader()
            .discover(tmp.path())
            .await
            .unwrap_or_else(|e| panic!("precompiling {fixture}: {e}"));
        let artifacts = std::fs::read_dir(tmp.path().join(".cache"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "cwasm"))
            .map(|path| {
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(&path).unwrap(),
                )
            })
            .collect();
        compiled.insert(fixture.to_owned(), artifacts);
    }
    let cache = plugins_dir.join(".cache");
    std::fs::create_dir_all(&cache).unwrap();
    for (name, bytes) in &compiled[fixture] {
        std::fs::write(cache.join(name), bytes).unwrap();
    }
}
