use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;

use infrarust_api::event::ResultedEvent;
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::connection::{
    KickedFromServerEvent, KickedFromServerResult, PlayerChooseInitialServerEvent,
    PlayerChooseInitialServerResult, ServerConnectedEvent, ServerPreConnectEvent,
    ServerPreConnectResult, ServerSwitchEvent,
};
use infrarust_api::events::lifecycle::{
    DisconnectCause, DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent,
    PermissionsSetupResult, PostLoginEvent, PreLoginEvent, PreLoginResult,
};
use infrarust_api::events::proxy::{
    ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent, ProxyShutdownEvent,
    ServerStateChangeEvent,
};
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::permissions::PermissionLevel;
use infrarust_api::player::Player;
use infrarust_api::plugin::Plugin;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{
    Component, GameProfile, HoverEvent, NamedColor, PlayerId, ProtocolVersion, ServerId,
};
use infrarust_core::event_bus::EventBusImpl;

use super::mock_services::MockPlayerRegistry;
use super::native_scripted::ScriptedPlugin;
use super::script::{self, EventName};
use super::{TestEnv, make_env, read_log, write_script};

pub const PLAYER: u64 = 1;
pub const USERNAME: &str = "Steve";
pub const UUID: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
pub const REMOTE: &str = "203.0.113.7:51234";
pub const DOMAIN: &str = "play.example.com";
pub const PROTOCOL: i32 = 767;
pub const MOTD: &str = "A Minecraft Proxy";
pub const KICK_REASON: &str = "Server closed";
pub const CHAT: &str = "hello";

pub const fn paired(native: &str, wasm: &str) -> bool {
    let native = native.as_bytes();
    let wasm = wasm.as_bytes();
    let native_suffix = b"_native";
    let wasm_suffix = b"_wasm";
    if native.len() <= native_suffix.len() || wasm.len() <= wasm_suffix.len() {
        return false;
    }
    let stem = native.len() - native_suffix.len();
    if wasm.len() - wasm_suffix.len() != stem {
        return false;
    }
    let mut i = 0;
    while i < native.len() {
        let expected = if i < stem {
            wasm[i]
        } else {
            native_suffix[i - stem]
        };
        if native[i] != expected {
            return false;
        }
        i += 1;
    }
    let mut j = 0;
    while j < wasm_suffix.len() {
        if wasm[stem + j] != wasm_suffix[j] {
            return false;
        }
        j += 1;
    }
    true
}

fn profile() -> GameProfile {
    GameProfile {
        uuid: UUID.parse().expect("canonical uuid"),
        username: USERNAME.to_owned(),
        properties: vec![],
    }
}

fn remote() -> SocketAddr {
    REMOTE.parse().expect("canonical address")
}

fn motd() -> Component {
    Component::text(MOTD).color(NamedColor::Gold).bold().append(
        Component::text(" v2")
            .color("#55ff55")
            .hover(HoverEvent::show_text("status")),
    )
}

fn player() -> PlayerId {
    PlayerId::new(PLAYER)
}

fn session() -> std::sync::Arc<dyn Player> {
    super::session_player(PLAYER, profile(), PROTOCOL, remote())
}

pub fn fields(event: EventName) -> Vec<String> {
    let player = PLAYER.to_string();
    let id = player.as_str();
    match event {
        EventName::PreLogin => owned(&[USERNAME, UUID, REMOTE, "767", DOMAIN]),
        EventName::PostLogin => owned(&[id, USERNAME, UUID, "767"]),
        EventName::Disconnect => owned(&[id, USERNAME, "lobby"]),
        EventName::OnlineAuthFailed => owned(&[USERNAME]),
        EventName::PermissionsSetup => owned(&[id, USERNAME, "true"]),
        EventName::ServerPreConnect => owned(&[id, USERNAME, "lobby"]),
        EventName::ServerConnected => owned(&[id, "lobby"]),
        EventName::ServerSwitch => owned(&[id, "lobby", "survival"]),
        EventName::KickedFromServer => {
            owned(&[id, "survival", &Component::text(KICK_REASON).to_json()])
        }
        EventName::PlayerChooseInitialServer => owned(&[id, USERNAME, "hub"]),
        EventName::ProxyPing => ping_fields(&motd().to_json()),
        EventName::ProxyInitialize | EventName::ProxyShutdown | EventName::ConfigReload => {
            Vec::new()
        }
        EventName::ServerStateChange => owned(&["survival", "starting", "online"]),
        EventName::ChatMessage => owned(&[id, CHAT]),
    }
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

pub fn ping_fields(description_json: &str) -> Vec<String> {
    owned(&[
        REMOTE,
        description_json,
        "100",
        "7",
        "767",
        "Infrarust",
        "-",
    ])
}

pub fn seen(event: EventName, priority: u8) -> String {
    seen_with(event, priority, &fields(event))
}

pub fn seen_with(event: EventName, priority: u8, fields: &[String]) -> String {
    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
    script::event_line(event, priority, &fields)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub summary: String,
    pub exact: String,
}

impl Outcome {
    fn same(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            summary: text.clone(),
            exact: text,
        }
    }

    fn component(tag: &str, component: &Component) -> Self {
        Self {
            summary: format!("{tag}:{}", component.to_plain()),
            exact: format!("{tag}:{}", component.to_json()),
        }
    }
}

fn limbo(handlers: &[String]) -> Outcome {
    Outcome::same(format!("limbo:{}", handlers.join(",")))
}

fn pre_login(result: &PreLoginResult) -> Outcome {
    match result {
        PreLoginResult::Allowed => Outcome::same("allowed"),
        PreLoginResult::Denied { reason } => Outcome::component("denied", reason),
        PreLoginResult::ForceOffline => Outcome::same("force-offline"),
        PreLoginResult::ForceOnline => Outcome::same("force-online"),
        _ => Outcome::same("unknown"),
    }
}

fn server_pre_connect(result: &ServerPreConnectResult) -> Outcome {
    match result {
        ServerPreConnectResult::Allowed => Outcome::same("allowed"),
        ServerPreConnectResult::ConnectTo(server) => {
            Outcome::same(format!("connect-to:{}", server.as_str()))
        }
        ServerPreConnectResult::SendToLimbo { limbo_handlers } => limbo(limbo_handlers),
        ServerPreConnectResult::Denied { reason } => Outcome::component("denied", reason),
        _ => Outcome::same("unknown"),
    }
}

fn kicked(result: &KickedFromServerResult) -> Outcome {
    match result {
        KickedFromServerResult::DisconnectPlayer { reason } => {
            Outcome::component("disconnect", reason)
        }
        KickedFromServerResult::RedirectTo(server) => {
            Outcome::same(format!("redirect:{}", server.as_str()))
        }
        KickedFromServerResult::SendToLimbo { limbo_handlers } => limbo(limbo_handlers),
        KickedFromServerResult::Notify { message } => Outcome::component("notify", message),
        _ => Outcome::same("unknown"),
    }
}

fn initial_server(result: &PlayerChooseInitialServerResult) -> Outcome {
    match result {
        PlayerChooseInitialServerResult::Allowed => Outcome::same("allowed"),
        PlayerChooseInitialServerResult::Redirect(server) => {
            Outcome::same(format!("redirect:{}", server.as_str()))
        }
        PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers } => limbo(limbo_handlers),
        _ => Outcome::same("unknown"),
    }
}

fn chat(result: &ChatMessageResult) -> Outcome {
    match result {
        ChatMessageResult::Allow => Outcome::same("allow"),
        ChatMessageResult::Deny { reason } => Outcome::component("deny", reason),
        ChatMessageResult::Modify { new_message } => Outcome::same(format!("modify:{new_message}")),
        _ => Outcome::same("unknown"),
    }
}

fn permissions(result: &PermissionsSetupResult) -> Outcome {
    match result {
        PermissionsSetupResult::UseDefault => Outcome::same("use-default"),
        PermissionsSetupResult::Custom(checker) => match checker.permission_level() {
            PermissionLevel::Admin => Outcome::same("custom:admin"),
            PermissionLevel::Player => Outcome::same("custom:player"),
        },
        _ => Outcome::same("unknown"),
    }
}

fn ping(response: &PingResponse) -> Outcome {
    Outcome {
        summary: format!("ping:{}", response.description.to_plain()),
        exact: format!(
            "ping:{}|{}|{}|{}|{}|{:?}",
            response.description.to_json(),
            response.max_players,
            response.online_players,
            response.protocol_version.raw(),
            response.version_name,
            response.favicon,
        ),
    }
}

pub async fn fire(bus: &EventBusImpl, event: EventName) -> Outcome {
    let protocol = ProtocolVersion::new(PROTOCOL);
    match event {
        EventName::PreLogin => {
            let event = PreLoginEvent::new(profile(), remote(), protocol, DOMAIN.to_owned());
            pre_login(bus.fire(event).await.result())
        }
        EventName::PostLogin => {
            bus.fire(PostLoginEvent::new(session())).await;
            Outcome::same("none")
        }
        EventName::Disconnect => {
            bus.fire(DisconnectEvent::new(
                session(),
                Some(ServerId::new("lobby")),
                DisconnectCause::ClientQuit,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::OnlineAuthFailed => {
            bus.fire(OnlineAuthFailed {
                username: USERNAME.to_owned(),
            })
            .await;
            Outcome::same("none")
        }
        EventName::PermissionsSetup => {
            let event = PermissionsSetupEvent::new(session(), true);
            permissions(bus.fire(event).await.result())
        }
        EventName::ServerPreConnect => {
            let event = ServerPreConnectEvent::new(player(), profile(), ServerId::new("lobby"));
            server_pre_connect(bus.fire(event).await.result())
        }
        EventName::ServerConnected => {
            bus.fire(ServerConnectedEvent {
                player_id: player(),
                server: ServerId::new("lobby"),
            })
            .await;
            Outcome::same("none")
        }
        EventName::ServerSwitch => {
            bus.fire(ServerSwitchEvent {
                player_id: player(),
                previous_server: ServerId::new("lobby"),
                new_server: ServerId::new("survival"),
            })
            .await;
            Outcome::same("none")
        }
        EventName::KickedFromServer => {
            let event = KickedFromServerEvent::new(
                player(),
                ServerId::new("survival"),
                Component::text(KICK_REASON),
            );
            kicked(bus.fire(event).await.result())
        }
        EventName::PlayerChooseInitialServer => {
            let event =
                PlayerChooseInitialServerEvent::new(player(), profile(), ServerId::new("hub"));
            initial_server(bus.fire(event).await.result())
        }
        EventName::ProxyPing => {
            let event = ProxyPingEvent {
                remote_addr: remote(),
                response: PingResponse::new(motd(), 100, 7, protocol, "Infrarust".to_owned(), None),
            };
            ping(&bus.fire(event).await.response)
        }
        EventName::ProxyInitialize => {
            bus.fire(ProxyInitializeEvent).await;
            Outcome::same("none")
        }
        EventName::ProxyShutdown => {
            bus.fire(ProxyShutdownEvent).await;
            Outcome::same("none")
        }
        EventName::ConfigReload => {
            bus.fire(ConfigReloadEvent).await;
            Outcome::same("none")
        }
        EventName::ServerStateChange => {
            bus.fire(ServerStateChangeEvent {
                server: ServerId::new("survival"),
                old_state: ServerState::Starting,
                new_state: ServerState::Online,
            })
            .await;
            Outcome::same("none")
        }
        EventName::ChatMessage => {
            let event = ChatMessageEvent::new(player(), CHAT.to_owned());
            chat(bus.fire(event).await.result())
        }
    }
}

enum StepKind {
    Fire(EventName),
    Command(&'static str),
    Disable,
}

struct Step {
    kind: StepKind,
    native: String,
    wasm: Option<String>,
}

#[derive(Default)]
pub struct Scenario {
    plugins: Vec<(&'static str, Vec<String>)>,
    steps: Vec<Step>,
    logs: BTreeMap<&'static str, Vec<String>>,
    wasm_logs: BTreeMap<&'static str, Vec<String>>,
    divergence: Option<&'static str>,
}

impl Scenario {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plugin<I, S>(mut self, id: &'static str, script: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.plugins
            .push((id, script.into_iter().map(Into::into).collect()));
        self
    }

    pub fn fire(self, event: EventName, expected: &str) -> Self {
        self.step(StepKind::Fire(event), expected, None)
    }

    pub fn fire_diverging(self, event: EventName, native: &str, wasm: &str) -> Self {
        self.step(StepKind::Fire(event), native, Some(wasm))
    }

    pub fn command(self, line: &'static str) -> Self {
        self.step(StepKind::Command(line), "found", None)
    }

    pub fn disable(self) -> Self {
        self.step(StepKind::Disable, "disabled", None)
    }

    pub fn log<I, S>(mut self, id: &'static str, lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.logs
            .insert(id, lines.into_iter().map(Into::into).collect());
        self
    }

    pub fn wasm_log<I, S>(mut self, id: &'static str, lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.wasm_logs
            .insert(id, lines.into_iter().map(Into::into).collect());
        self
    }

    pub fn expect_divergence(mut self, reason: &'static str) -> Self {
        self.divergence = Some(reason);
        self
    }

    fn step(mut self, kind: StepKind, native: &str, wasm: Option<&str>) -> Self {
        self.steps.push(Step {
            kind,
            native: native.to_owned(),
            wasm: wasm.map(str::to_owned),
        });
        self
    }

    fn validate(&self) {
        let overrides = !self.wasm_logs.is_empty() || self.steps.iter().any(|s| s.wasm.is_some());
        assert_eq!(
            overrides,
            self.divergence.is_some(),
            "WASM-specific expectations and expect_divergence go together"
        );
        for id in self.logs.keys().chain(self.wasm_logs.keys()) {
            assert!(
                self.plugins.iter().any(|(plugin, _)| plugin == id),
                "log expectation for unknown plugin {id}"
            );
        }
    }

    fn expected_results(&self, side: Side) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| match side {
                Side::Wasm => step.wasm.clone().unwrap_or_else(|| step.native.clone()),
                Side::Native => step.native.clone(),
            })
            .collect()
    }

    fn expected_logs(&self, side: Side) -> BTreeMap<String, Vec<String>> {
        self.plugins
            .iter()
            .map(|(id, _)| {
                let lines = match side {
                    Side::Wasm => self.wasm_logs.get(id).or_else(|| self.logs.get(id)),
                    Side::Native => self.logs.get(id),
                };
                let mut log = vec!["enable".to_owned()];
                log.extend(lines.into_iter().flatten().cloned());
                ((*id).to_owned(), log)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Native,
    Wasm,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Observed {
    pub results: Vec<Outcome>,
    pub logs: BTreeMap<String, Vec<String>>,
}

impl Observed {
    fn summaries(&self) -> Vec<String> {
        self.results.iter().map(|o| o.summary.clone()).collect()
    }
}

async fn drive(
    env: &TestEnv,
    plugins_dir: &Path,
    plugins: &[Box<dyn Plugin>],
    scenario: &Scenario,
) -> Observed {
    let mut results = Vec::new();
    for step in &scenario.steps {
        let outcome = match step.kind {
            StepKind::Fire(event) => fire(&env.event_bus, event).await,
            StepKind::Command(line) => {
                let found = env
                    .command_manager
                    .dispatch(None, line, &MockPlayerRegistry)
                    .await;
                Outcome::same(if found { "found" } else { "missing" })
            }
            StepKind::Disable => {
                for plugin in plugins {
                    plugin.on_disable().await.expect("on_disable");
                }
                Outcome::same("disabled")
            }
        };
        results.push(outcome);
    }
    let logs = scenario
        .plugins
        .iter()
        .map(|(id, _)| ((*id).to_owned(), read_log(&plugins_dir.join(id))))
        .collect();
    Observed { results, logs }
}

fn write_scripts(plugins_dir: &Path, scenario: &Scenario) {
    for (id, lines) in &scenario.plugins {
        write_script(plugins_dir, id, &lines.join("\n"));
    }
}

pub async fn observe_native(scenario: &Scenario) -> Observed {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plugins_dir = tmp.path().to_path_buf();
    write_scripts(&plugins_dir, scenario);
    let env = make_env(plugins_dir.clone());
    let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();
    for (id, _) in &scenario.plugins {
        let plugin = ScriptedPlugin::new(id);
        let ctx = env.factory.create_context(id);
        plugin
            .on_enable(ctx.as_ref())
            .await
            .unwrap_or_else(|e| panic!("enable native {id}: {e}"));
        plugins.push(Box::new(plugin));
    }
    drive(&env, &plugins_dir, &plugins, scenario).await
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub async fn observe_wasm(scenario: &Scenario) -> Observed {
    use infrarust_api::loader::PluginLoader;

    let tmp = tempfile::tempdir().expect("tempdir");
    let plugins_dir = tmp.path().to_path_buf();
    for (id, _) in &scenario.plugins {
        super::add_precompiled_fixture(&plugins_dir, id).await;
    }
    write_scripts(&plugins_dir, scenario);
    let env = make_env(plugins_dir.clone());
    let loader = super::fresh_loader();
    loader.discover(&plugins_dir).await.expect("discover");
    let mut plugins = Vec::new();
    for (id, _) in &scenario.plugins {
        plugins.push(super::load_enabled(&loader, &env.factory, id).await);
    }
    drive(&env, &plugins_dir, &plugins, scenario).await
}

pub async fn check_native(scenario: Scenario) {
    scenario.validate();
    let native = observe_native(&scenario).await;
    assert_eq!(
        native.summaries(),
        scenario.expected_results(Side::Native),
        "native event results"
    );
    assert_eq!(
        native.logs,
        scenario.expected_logs(Side::Native),
        "native plugin logs"
    );
}

#[cfg(all(feature = "wasm", wasm_fixtures_available))]
pub async fn check_wasm(scenario: Scenario) {
    scenario.validate();
    let wasm = observe_wasm(&scenario).await;
    assert_eq!(
        wasm.summaries(),
        scenario.expected_results(Side::Wasm),
        "WASM event results"
    );
    assert_eq!(
        wasm.logs,
        scenario.expected_logs(Side::Wasm),
        "WASM plugin logs"
    );
    let native = observe_native(&scenario).await;
    match scenario.divergence {
        None => {
            assert_eq!(
                wasm.results, native.results,
                "WASM and native results must match exactly"
            );
            assert_eq!(wasm.logs, native.logs, "WASM and native logs must match");
        }
        Some(reason) => assert_ne!(
            wasm, native,
            "the documented divergence no longer reproduces, flip the scenario: {reason}"
        ),
    }
}
