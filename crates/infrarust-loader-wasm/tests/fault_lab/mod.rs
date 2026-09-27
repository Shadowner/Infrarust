#![allow(dead_code)]

#[path = "../fixtures/fault-lab/src/faults.rs"]
pub mod faults;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use infrarust_api::event::Event;
use infrarust_api::events::chat::ChatMessageEvent;
use infrarust_api::events::connection::ServerPreConnectEvent;
use infrarust_api::events::lifecycle::{DisconnectEvent, PostLoginEvent, PreLoginEvent};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::events::proxy::ProxyPingEvent;
use infrarust_api::limbo::{LimboEntryContext, LimboHandler};
use infrarust_api::loader::{PluginContextFactory, PluginLoader};
use infrarust_api::plugin::{Plugin, PluginContext};
use infrarust_api::test_util::{MockBanService, MockConfigService, MockPlayerRegistry};
use infrarust_api::test_util::RecordingLimboSession;
use infrarust_api::types::{PlayerId, ProtocolVersion, ServerId};
use infrarust_core::ban::BanManager;
use infrarust_core::event_bus::{EventBusConfig, EventBusImpl};
use infrarust_core::filter::codec_registry::CodecFilterRegistryImpl;
use infrarust_core::permissions::PermissionService;
use infrarust_core::plugin::manager::PluginServices;
use infrarust_core::plugin::{PluginContextFactoryImpl, PluginPermissions};
use infrarust_core::services::command_manager::{CommandManagerImpl, DispatchOutcome};
use infrarust_core::services::scheduler::SchedulerImpl;
use infrarust_loader_wasm::WasmPluginLoader;

use crate::support;


pub const LAB: &str = "fault-lab";
pub const PEER: &str = "fault-lab-peer";
pub const PROMPTLY: Duration = Duration::from_secs(15);

pub struct LabPlugin {
    pub fixture: &'static str,
    pub id: &'static str,
    pub faults: String,
    pub grants: Vec<&'static str>,
}

impl LabPlugin {
    pub fn lab(faults: &str) -> Self {
        Self::fixture("fault-lab", LAB, faults)
    }

    pub fn peer(faults: &str) -> Self {
        Self::fixture("fault-lab-peer", PEER, &format!("prefix peer\n{faults}"))
    }

    pub fn fixture(fixture: &'static str, id: &'static str, faults: &str) -> Self {
        Self {
            fixture,
            id,
            faults: faults.to_owned(),
            grants: vec!["chat-intercept"],
        }
    }

    pub fn grant(mut self, permission: &'static str) -> Self {
        self.grants.push(permission);
        self
    }
}

#[derive(Default)]
pub struct LabOptions {
    pub proxy_toml: String,
    pub bus: Option<EventBusConfig>,
    pub ban_manager: Option<Arc<BanManager>>,
    pub permissions: Option<Arc<PermissionService>>,
    pub extra: Vec<(&'static str, &'static str, String)>,
}

pub struct Lab {
    pub tmp: tempfile::TempDir,
    pub plugins_dir: PathBuf,
    pub event_bus: Arc<EventBusImpl>,
    pub commands: Arc<CommandManagerImpl>,
    pub codecs: Arc<CodecFilterRegistryImpl>,
    pub scheduler: Arc<SchedulerImpl>,
    pub factory: PluginContextFactoryImpl,
    pub loader: WasmPluginLoader,
    pub plugins: Mutex<HashMap<String, Box<dyn Plugin>>>,
    pub marks: Mutex<HashMap<String, usize>>,
}

impl Lab {
    pub async fn start(plugins: Vec<LabPlugin>, options: LabOptions) -> Self {
        let lab = Self::stage(&plugins, options).await;
        for plugin in &plugins {
            lab.enable(plugin.id).await;
        }
        lab
    }

    pub async fn stage(plugins: &[LabPlugin], options: LabOptions) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let plugins_dir = tmp.path().to_path_buf();
        let mut grants: HashMap<String, PluginPermissions> = HashMap::new();
        for plugin in plugins {
            support::add_precompiled_fixture(&plugins_dir, plugin.fixture).await;
            let data = plugins_dir.join(plugin.id);
            std::fs::create_dir_all(&data).unwrap();
            std::fs::write(data.join(faults::FAULTS_FILE), &plugin.faults).unwrap();
            let entry = grants.entry(plugin.id.to_owned()).or_default();
            for grant in &plugin.grants {
                entry.permissions.push((*grant).to_owned());
            }
        }
        for (fixture, id, file) in &options.extra {
            support::add_precompiled_fixture(&plugins_dir, fixture).await;
            let data = plugins_dir.join(id);
            std::fs::create_dir_all(&data).unwrap();
            if !file.is_empty() {
                std::fs::write(data.join(support::script::SCRIPT_FILE), file).unwrap();
            }
            grants
                .entry((*id).to_owned())
                .or_default()
                .permissions
                .push("chat-intercept".to_owned());
        }
        let event_bus = Arc::new(EventBusImpl::with_config(options.bus.unwrap_or_default()));
        let commands = Arc::new(CommandManagerImpl::new());
        let codecs = Arc::new(CodecFilterRegistryImpl::new());
        let scheduler = Arc::new(SchedulerImpl::new());
        let ban_service: Arc<dyn infrarust_api::services::ban_service::BanService> =
            match &options.ban_manager {
                Some(manager) => Arc::clone(manager) as _,
                None => Arc::new(MockBanService::new()),
            };
        let services = PluginServices {
            player_registry: Arc::new(MockPlayerRegistry::new()),
            ban_service,
            command_manager: Arc::clone(&commands),
            config_service: Arc::new(MockConfigService::new()),
            codec_filter_registry: Arc::clone(&codecs),
            scheduler: Arc::clone(&scheduler),
            plugins_dir: plugins_dir.clone(),
            ..PluginServices::for_tests_with(Arc::clone(&event_bus))
        };
        let mut factory = PluginContextFactoryImpl::new(services, grants);
        if let Some(manager) = options.ban_manager {
            factory = factory.with_ban_manager(manager);
        }
        if let Some(permissions) = options.permissions {
            factory = factory.with_permissions(permissions);
        }
        let loader = support::loader_from_toml(&options.proxy_toml);
        loader.discover(&plugins_dir).await.unwrap();
        Self {
            tmp,
            plugins_dir,
            event_bus,
            commands,
            codecs,
            scheduler,
            factory,
            loader,
            plugins: Mutex::new(HashMap::new()),
            marks: Mutex::new(HashMap::new()),
        }
    }

    pub async fn enable(&self, id: &str) {
        let plugin = support::load_enabled(&self.loader, &self.factory, id).await;
        self.plugins.lock().unwrap().insert(id.to_owned(), plugin);
    }

    pub fn take(&self, id: &str) -> Box<dyn Plugin> {
        self.plugins
            .lock()
            .unwrap()
            .remove(id)
            .unwrap_or_else(|| panic!("{id} is not enabled"))
    }

    pub async fn load(&self, id: &str) -> Box<dyn Plugin> {
        self.loader
            .load(id, &self.factory)
            .await
            .unwrap_or_else(|e| panic!("load {id}: {e}"))
    }

    pub fn context(&self, id: &str) -> Arc<dyn PluginContext> {
        self.factory.create_context(id)
    }

    pub fn data(&self, id: &str) -> PathBuf {
        self.plugins_dir.join(id)
    }

    pub fn set_faults(&self, id: &str, faults: &str) {
        std::fs::write(self.data(id).join(faults::FAULTS_FILE), faults).unwrap();
    }

    pub fn log(&self, id: &str) -> Vec<String> {
        read_lines(&self.data(id).join(faults::LOG_FILE))
    }

    pub fn mark(&self, id: &str) {
        let lines = self.log(id).len();
        self.marks.lock().unwrap().insert(id.to_owned(), lines);
    }

    pub fn since_mark(&self, id: &str) -> Vec<String> {
        let start = self.marks.lock().unwrap().get(id).copied().unwrap_or(0);
        self.log(id).into_iter().skip(start).collect()
    }

    pub fn clear_log(&self, id: &str) {
        let _ = std::fs::remove_file(self.data(id).join(faults::LOG_FILE));
    }

    pub fn listeners(&self, id: &str) -> usize {
        owned::<PreLoginEvent>(&self.event_bus, id)
            + owned::<PostLoginEvent>(&self.event_bus, id)
            + owned::<ChatMessageEvent>(&self.event_bus, id)
            + owned::<ServerPreConnectEvent>(&self.event_bus, id)
            + owned::<ProxyPingEvent>(&self.event_bus, id)
            + owned::<DisconnectEvent>(&self.event_bus, id)
            + owned::<NamedEvent>(&self.event_bus, id)
    }

    pub fn tasks(&self, id: &str) -> usize {
        self.scheduler.owned_count(id)
    }

    pub fn command_names(&self, id: &str) -> Vec<String> {
        self.commands
            .commands_for_plugin(id)
            .iter()
            .map(|info| info.name().to_owned())
            .collect()
    }

    pub fn limbo_handlers(&self, id: &str) -> Vec<Arc<dyn LimboHandler>> {
        self.factory.context(id).limbo_handlers()
    }

    pub fn limbo_handler(&self, id: &str, name: &str) -> Arc<dyn LimboHandler> {
        self.limbo_handlers(id)
            .into_iter()
            .find(|handler| handler.name() == name)
            .unwrap_or_else(|| panic!("{id} has no limbo handler {name}"))
    }

    pub fn channels(&self, id: &str) -> usize {
        self.context(id).channel_registrar().channels().len()
    }

    pub async fn dispatch(&self, line: &str) -> Duration {
        let started = Instant::now();
        let outcome = tokio::time::timeout(
            PROMPTLY,
            self.commands.dispatch(support::console(), line),
        )
        .await
        .unwrap_or_else(|_| panic!("`{line}` did not return within {PROMPTLY:?}"));
        assert_eq!(outcome, DispatchOutcome::Executed, "{line}");
        started.elapsed()
    }

    pub async fn complete(&self, input: &str) -> Vec<String> {
        tokio::time::timeout(PROMPTLY, self.commands.suggest(support::console(), input))
            .await
            .unwrap_or_else(|_| panic!("completing `{input}` did not return within {PROMPTLY:?}"))
            .unwrap_or_default()
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect()
    }

    pub async fn wait_for(&self, what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + PROMPTLY;
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

}

pub fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

pub fn owned<E: Event>(bus: &EventBusImpl, id: &str) -> usize {
    bus.listener_owners::<E>()
        .iter()
        .filter(|owner| owner.as_ref() == id)
        .count()
}

pub fn player(id: u64) -> Arc<dyn infrarust_api::player::Player> {
    support::session_player(
        id,
        support::nil_profile("Steve"),
        ProtocolVersion::MINECRAFT_1_21.raw(),
        "127.0.0.1:40000".parse().unwrap(),
    )
}

pub fn chat() -> ChatMessageEvent {
    ChatMessageEvent::new(
        player(1),
        "hello".to_owned(),
        false,
        Some(ServerId::from("lobby")),
    )
}

pub fn pre_login() -> PreLoginEvent {
    PreLoginEvent::new(
        support::nil_profile("Steve"),
        SocketAddr::from(([127, 0, 0, 1], 25565)),
        ProtocolVersion::MINECRAFT_1_21,
        "play.example.com".to_owned(),
    )
}

pub fn limbo_session(player_id: u64) -> Arc<RecordingLimboSession> {
    RecordingLimboSession::new(
        PlayerId::new(player_id),
        support::nil_profile("Tester"),
        LimboEntryContext::InitialConnection {
            target_server: ServerId::from("hub"),
        },
    )
}

pub fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kib| kib.parse().ok())
        .unwrap_or(0)
}

pub fn alive_tasks() -> usize {
    tokio::runtime::Handle::current().metrics().num_alive_tasks()
}

pub fn count(lines: &[String], wanted: &str) -> usize {
    lines.iter().filter(|line| line.as_str() == wanted).count()
}

pub fn count_prefix(lines: &[String], prefix: &str) -> usize {
    lines.iter().filter(|line| line.starts_with(prefix)).count()
}
