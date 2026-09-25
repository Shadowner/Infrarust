use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use infrarust_api::event::{ConnectionState, ResultedEvent};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
use infrarust_api::events::command::{CommandExecuteEvent, CommandExecuteResult};
use infrarust_api::events::connection::{
    ConnectCause, KickCause, KickedFromServerEvent, KickedFromServerResult,
    PlayerChooseInitialServerEvent, PlayerChooseInitialServerResult, ServerConnectedEvent,
    ServerPostConnectEvent, ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, HandshakeIntent,
    RejectReason,
};
use infrarust_api::events::lifecycle::{
    DisconnectCause, DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult,
    OnlineAuthFailed, PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent, PreLoginEvent,
    PreLoginResult,
};
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
use infrarust_api::events::messaging::{PluginMessageEvent, PluginMessageResult};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::events::packet::{PacketDirection, RawPacketEvent, RawPacketResult};
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, PingResponse, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::events::resource_pack::{PlayerResourcePackStatusEvent, ResourcePackOrigin};
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult, TransferOrigin};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::loader::PluginContextFactory;
use infrarust_api::messaging::{ChannelId, Endpoint, MessagePhase};
use infrarust_api::permissions::ADMIN_PERMISSION;
use infrarust_api::player::Player;
use infrarust_api::player::{ClientSettings, MainHand, ResourcePackStatus};
use infrarust_api::plugin::Plugin;
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::services::load_balancer::BackendState;
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{
    Component, GameProfile, HoverEvent, NamedColor, ProtocolVersion, ServerAddress, ServerId,
};
use infrarust_api::types::{PlayerId, RawPacket};
use infrarust_core::event_bus::EventBusImpl;
use infrarust_core::services::command_manager::DispatchOutcome;

use super::native_scripted::ScriptedPlugin;
use super::script::{self, EventName};
use super::{EnvOptions, TestEnv, make_env_with, read_log, write_script};

pub const PLAYER: u64 = 1;
pub const USERNAME: &str = "Steve";
pub const UUID: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
pub const REMOTE: &str = "203.0.113.7:51234";
pub const DOMAIN: &str = "play.example.com";
pub const PROTOCOL: i32 = 767;
pub const MOTD: &str = "A Minecraft Proxy";
pub const KICK_REASON: &str = "Server closed";
pub const CHAT: &str = "hello";
pub const COMMAND: &str = "spawn now";
pub const BAN_ID: &str = "ban-1";
pub const PACK: &str = "00000000-0000-0000-0000-000000000005";
pub const NAMED: &str = "echo";

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

fn session() -> std::sync::Arc<dyn Player> {
    super::session_player(PLAYER, profile(), PROTOCOL, remote())
}

pub fn fields(event: EventName) -> Vec<String> {
    let player = PLAYER.to_string();
    let id = player.as_str();
    match event {
        EventName::PreLogin => owned(&[USERNAME, UUID, REMOTE, "767", DOMAIN]),
        EventName::PostLogin => owned(&[id, USERNAME, UUID, "767"]),
        EventName::Disconnect => owned(&[id, USERNAME, "lobby", "client_quit"]),
        EventName::OnlineAuthFailed => owned(&[USERNAME]),
        EventName::PermissionsSetup => owned(&[id, USERNAME, "true"]),
        EventName::ServerPreConnect => owned(&[id, USERNAME, "lobby", "hub", "switch"]),
        EventName::ServerConnected => owned(&[id, "lobby", "-"]),
        EventName::ServerPostConnect => owned(&[id, "survival", "lobby"]),
        EventName::KickedFromServer => owned(&[
            id,
            "survival",
            &Component::text(KICK_REASON).to_json(),
            "play_disconnect",
            "false",
            "lobby",
        ]),
        EventName::PlayerChooseInitialServer => owned(&[id, USERNAME, "hub"]),
        EventName::ProxyPing => ping_fields(&motd().to_json()),
        EventName::ProxyInitialize | EventName::ProxyShutdown => Vec::new(),
        EventName::ConfigReload => owned(&["file", "survival", "-", "lobby"]),
        EventName::ServerStateChange => owned(&["survival", "starting", "online"]),
        EventName::ChatMessage => owned(&[id, CHAT, "false", "lobby"]),
        EventName::BackendHealth => owned(&["10.0.0.2:25565", "lobby,survival", "draining"]),
        EventName::Login => owned(&[id, USERNAME, "true"]),
        EventName::GameProfileRequest => {
            owned(&[USERNAME, UUID, "false", REMOTE, DOMAIN, "767", USERNAME])
        }
        EventName::CommandExecute => owned(&[id, COMMAND, "true", "lobby"]),
        EventName::ConnectionHandshake => owned(&[
            REMOTE,
            DOMAIN,
            "Play.Example.Com\0FML3\0",
            "25565",
            "767",
            "login",
            "false",
            "lobby",
        ]),
        EventName::ConnectionRejected => owned(&[REMOTE, DOMAIN, "plugin:gate"]),
        EventName::LimboEnter => owned(&[id, "gate,queue", "kicked:survival"]),
        EventName::LimboExit => owned(&[id, "sent_to_limbo", "lobby"]),
        EventName::PlayerClientBrand => owned(&[id, "fabric"]),
        EventName::PlayerSettingsChanged => owned(&[id, "fr_fr", "12", "left"]),
        EventName::PlayerChannelRegister => owned(&[id, "test:echo,mod:b", "serverbound"]),
        EventName::PluginMessage => owned(&[
            id,
            "backend:lobby",
            "bungeecord:main",
            "BungeeCord",
            "BungeeCord",
            "payload",
            "play",
        ]),
        EventName::BanIssued => {
            owned(&[BAN_ID, "username:Steve", "griefing", "player:Admin", "true"])
        }
        EventName::BanRevoked => owned(&[BAN_ID, "username:Steve", "-", "web-api:ops", "false"]),
        EventName::PluginEnabled => owned(&["stats", "1.2.0"]),
        EventName::PluginDisabled => owned(&["stats"]),
        EventName::PreTransfer => owned(&[id, "old.example.com", "25565", "backend"]),
        EventName::PlayerResourcePackStatus => owned(&[id, PACK, "declined", "proxy"]),
        EventName::NamedEvent => owned(&[NAMED, "-", "text/plain", "ping", "false", "-"]),
        EventName::RawPacket => owned(&[id, "serverbound", "5", "abc"]),
    }
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

pub fn ping_fields(description_json: &str) -> Vec<String> {
    owned(&[
        REMOTE,
        "lobby",
        DOMAIN,
        "767",
        "false",
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
        KickedFromServerResult::DisconnectPlayer {
            reason: Some(reason),
        } => Outcome::component("disconnect", reason),
        KickedFromServerResult::DisconnectPlayer { reason: None } => Outcome::same("disconnect"),
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
        ChatMessageResult::Deny {
            reason: Some(reason),
        } => Outcome::component("deny", reason),
        ChatMessageResult::Deny { reason: None } => Outcome::same("deny"),
        ChatMessageResult::Modify { message } => Outcome::same(format!("modify:{message}")),
        _ => Outcome::same("unknown"),
    }
}

fn permissions(result: &PermissionsSetupResult) -> Outcome {
    match result {
        PermissionsSetupResult::UseDefault => Outcome::same("use-default"),
        PermissionsSetupResult::Custom(checker) if checker.has_permission(ADMIN_PERMISSION) => {
            Outcome::same("custom:admin")
        }
        PermissionsSetupResult::Custom(_) => Outcome::same("custom:player"),
        _ => Outcome::same("unknown"),
    }
}

fn login(result: &LoginResult) -> Outcome {
    match result {
        LoginResult::Allowed => Outcome::same("allowed"),
        LoginResult::Denied { reason } => Outcome::component("denied", reason),
        _ => Outcome::same("unknown"),
    }
}

fn command(result: &CommandExecuteResult) -> Outcome {
    match result {
        CommandExecuteResult::Allow => Outcome::same("allow"),
        CommandExecuteResult::Deny {
            reason: Some(reason),
        } => Outcome::component("deny", reason),
        CommandExecuteResult::Deny { reason: None } => Outcome::same("deny"),
        CommandExecuteResult::Modify { command } => Outcome::same(format!("modify:{command}")),
        CommandExecuteResult::ForwardToBackend => Outcome::same("forward-to-backend"),
        _ => Outcome::same("unknown"),
    }
}

fn handshake(result: &ConnectionHandshakeResult) -> Outcome {
    match result {
        ConnectionHandshakeResult::Allow => Outcome::same("allow"),
        ConnectionHandshakeResult::Deny {
            reason: Some(reason),
        } => Outcome::component("deny", reason),
        ConnectionHandshakeResult::Deny { reason: None } => Outcome::same("deny"),
        ConnectionHandshakeResult::DropSilently => Outcome::same("drop"),
        _ => Outcome::same("unknown"),
    }
}

fn plugin_message(result: &PluginMessageResult) -> Outcome {
    match result {
        PluginMessageResult::Forward => Outcome::same("forward"),
        PluginMessageResult::Handled => Outcome::same("handled"),
        PluginMessageResult::Replace(data) => {
            Outcome::same(format!("replace:{}", String::from_utf8_lossy(data)))
        }
        _ => Outcome::same("unknown"),
    }
}

fn transfer(result: &PreTransferResult) -> Outcome {
    match result {
        PreTransferResult::Allowed => Outcome::same("allowed"),
        PreTransferResult::Denied { reason } => Outcome::component("denied", reason),
        PreTransferResult::Redirect { host, port } => {
            Outcome::same(format!("redirect:{host}:{port}"))
        }
        _ => Outcome::same("unknown"),
    }
}

fn named(event: &NamedEvent) -> Outcome {
    let response = event.response.as_ref().map_or_else(
        || "-".to_owned(),
        |response| {
            format!(
                "{}={}",
                response.content_type,
                String::from_utf8_lossy(&response.payload)
            )
        },
    );
    Outcome::same(format!("named:{}:{response}", event.cancelled))
}

fn raw_packet(result: &RawPacketResult) -> Outcome {
    match result {
        RawPacketResult::Pass => Outcome::same("pass"),
        RawPacketResult::Drop => Outcome::same("drop"),
        RawPacketResult::Modify { packet } => Outcome::same(format!(
            "modify:{}:{}",
            packet.packet_id,
            String::from_utf8_lossy(&packet.data)
        )),
        _ => Outcome::same("unknown"),
    }
}

fn ban_entry() -> BanEntry {
    BanEntry::new(
        BAN_ID,
        BanTarget::Username(USERNAME.to_owned()),
        BanSource::Console,
    )
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
            let event = ServerPreConnectEvent::new(
                session(),
                ServerId::new("lobby"),
                Some(ServerId::new("hub")),
                ConnectCause::Switch,
            );
            server_pre_connect(bus.fire(event).await.result())
        }
        EventName::ServerConnected => {
            bus.fire(ServerConnectedEvent::new(
                session(),
                ServerId::new("lobby"),
                None,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::ServerPostConnect => {
            bus.fire(ServerPostConnectEvent::new(
                session(),
                ServerId::new("survival"),
                Some(ServerId::new("lobby")),
            ))
            .await;
            Outcome::same("none")
        }
        EventName::KickedFromServer => {
            let event = KickedFromServerEvent::new(
                session(),
                ServerId::new("survival"),
                Some(Component::text(KICK_REASON)),
                KickCause::PlayDisconnect,
                false,
                Some(ServerId::new("lobby")),
                KickedFromServerResult::default(),
            );
            kicked(bus.fire(event).await.result())
        }
        EventName::PlayerChooseInitialServer => {
            let event = PlayerChooseInitialServerEvent::new(session(), ServerId::new("hub"));
            initial_server(bus.fire(event).await.result())
        }
        EventName::ProxyPing => {
            let event = ProxyPingEvent::new(
                remote(),
                Some(ServerId::new("lobby")),
                Some(DOMAIN.to_owned()),
                protocol,
                false,
                PingResponse::new(motd(), 100, 7, protocol, "Infrarust".to_owned(), None),
            );
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
            bus.fire(ConfigReloadEvent::new(
                "file",
                vec![ServerId::new("survival")],
                Vec::new(),
                vec![ServerId::new("lobby")],
            ))
            .await;
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
            let event = ChatMessageEvent::new(
                session(),
                CHAT.to_owned(),
                false,
                Some(ServerId::new("lobby")),
            );
            chat(bus.fire(event).await.result())
        }
        EventName::BackendHealth => {
            bus.fire(BackendHealthEvent {
                address: ServerAddress {
                    host: "10.0.0.2".to_owned(),
                    port: 25565,
                },
                servers: vec![ServerId::new("lobby"), ServerId::new("survival")],
                state: BackendState::Draining,
            })
            .await;
            Outcome::same("none")
        }
        EventName::Login => login(bus.fire(LoginEvent::new(session(), true)).await.result()),
        EventName::GameProfileRequest => {
            let event = GameProfileRequestEvent::new(
                profile(),
                false,
                remote(),
                Some(DOMAIN.to_owned()),
                protocol,
            );
            let event = bus.fire(event).await;
            Outcome::same(format!("profile:{}", event.profile.username))
        }
        EventName::CommandExecute => {
            let event = CommandExecuteEvent::new(
                session(),
                COMMAND.to_owned(),
                true,
                Some(ServerId::new("lobby")),
            );
            command(bus.fire(event).await.result())
        }
        EventName::ConnectionHandshake => {
            let event = ConnectionHandshakeEvent::new(remote(), HandshakeIntent::Login, protocol)
                .with_host("Play.Example.Com\0FML3\0", Some(DOMAIN.to_owned()), 25565)
                .with_server(Some(ServerId::new("lobby")));
            handshake(bus.fire(event).await.result())
        }
        EventName::ConnectionRejected => {
            bus.fire(ConnectionRejectedEvent::new(
                remote(),
                Some(DOMAIN.to_owned()),
                RejectReason::Plugin {
                    plugin_id: Some("gate".to_owned()),
                },
            ))
            .await;
            Outcome::same("none")
        }
        EventName::LimboEnter => {
            bus.fire(LimboEnterEvent::new(
                session(),
                vec!["gate".to_owned(), "queue".to_owned()],
                LimboEntryContext::KickedFromServer {
                    server: ServerId::new("survival"),
                    reason: Component::text(KICK_REASON),
                },
            ))
            .await;
            Outcome::same("none")
        }
        EventName::LimboExit => {
            bus.fire(LimboExitEvent::new(
                session(),
                LimboExitReason::SentToLimbo {
                    handlers: vec!["queue".to_owned()],
                },
                Some(ServerId::new("lobby")),
            ))
            .await;
            Outcome::same("none")
        }
        EventName::PlayerClientBrand => {
            bus.fire(PlayerClientBrandEvent::new(session(), "fabric".to_owned()))
                .await;
            Outcome::same("none")
        }
        EventName::PlayerSettingsChanged => {
            let mut settings = ClientSettings::new("fr_fr");
            settings.view_distance = 12;
            settings.main_hand = MainHand::Left;
            bus.fire(PlayerSettingsChangedEvent::new(session(), settings))
                .await;
            Outcome::same("none")
        }
        EventName::PlayerChannelRegister => {
            bus.fire(PlayerChannelRegisterEvent::new(
                session(),
                vec!["test:echo".to_owned(), "mod:b".to_owned()],
                PacketDirection::Serverbound,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::PluginMessage => {
            let event = PluginMessageEvent::new(
                session(),
                Endpoint::Backend(ServerId::new("lobby")),
                ChannelId::bungeecord(),
                "BungeeCord".to_owned(),
                Bytes::from_static(b"payload"),
                MessagePhase::Play,
            );
            plugin_message(bus.fire(event).await.result())
        }
        EventName::BanIssued => {
            bus.fire(BanIssuedEvent::new(
                ban_entry().reason("griefing"),
                BanSource::Player {
                    uuid: uuid::Uuid::from_u128(9),
                    name: "Admin".to_owned(),
                },
                true,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::BanRevoked => {
            bus.fire(BanRevokedEvent::new(
                ban_entry(),
                BanSource::WebApi {
                    actor: Some("ops".to_owned()),
                },
                false,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::PluginEnabled => {
            bus.fire(PluginEnabledEvent::new("stats", "1.2.0")).await;
            Outcome::same("none")
        }
        EventName::PluginDisabled => {
            bus.fire(PluginDisabledEvent::new("stats")).await;
            Outcome::same("none")
        }
        EventName::PreTransfer => {
            let event = PreTransferEvent::new(
                session(),
                "old.example.com".to_owned(),
                25565,
                TransferOrigin::Backend,
            );
            transfer(bus.fire(event).await.result())
        }
        EventName::PlayerResourcePackStatus => {
            bus.fire(PlayerResourcePackStatusEvent::new(
                session(),
                Some(PACK.parse().expect("canonical uuid")),
                ResourcePackStatus::Declined,
                ResourcePackOrigin::Proxy,
            ))
            .await;
            Outcome::same("none")
        }
        EventName::NamedEvent => {
            named(&bus.fire(NamedEvent::new(NAMED, "text/plain", "ping")).await)
        }
        EventName::RawPacket => {
            let mut event = RawPacketEvent::new(
                PlayerId::new(PLAYER),
                PacketDirection::Serverbound,
                RawPacket::new(script::PACKET_ID, Bytes::from_static(b"abc")),
            );
            bus.fire_packet_event(
                script::PACKET_ID,
                ConnectionState::Play,
                PacketDirection::Serverbound,
                &mut event,
            )
            .await;
            raw_packet(event.result())
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
    grants: Vec<(&'static str, &'static str)>,
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

    pub fn grant(mut self, id: &'static str, capability: &'static str) -> Self {
        self.grants.push((id, capability));
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
                let found = env.command_manager.dispatch(super::console(), line).await
                    == DispatchOutcome::Executed;
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

fn environment(plugins_dir: PathBuf, scenario: &Scenario) -> TestEnv {
    let options = scenario
        .grants
        .iter()
        .fold(EnvOptions::default(), |options, (id, capability)| {
            options.grant(id, capability)
        });
    make_env_with(plugins_dir, options)
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
    let env = environment(plugins_dir.clone(), scenario);
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
    let env = environment(plugins_dir.clone(), scenario);
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
