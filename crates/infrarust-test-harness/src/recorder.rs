use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use infrarust_api::error::PluginError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, Event, EventPriority, ResultedEvent};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::command::{CommandExecuteEvent, CommandExecuteResult};
use infrarust_api::events::connection::{
    KickedFromServerEvent, KickedFromServerResult, PlayerChooseInitialServerEvent,
    PlayerChooseInitialServerResult, ServerConnectedEvent, ServerPostConnectEvent,
    ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::handshake::{
    ConnectionHandshakeEvent, ConnectionHandshakeResult, ConnectionRejectedEvent, RejectReason,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, GameProfileRequestEvent, LoginEvent, LoginResult, OnlineAuthFailed,
    PermissionsSetupEvent, PermissionsSetupResult, PostLoginEvent, PreLoginEvent, PreLoginResult,
};
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent, LimboExitReason};
use infrarust_api::events::plugin::{
    PluginDisabledEvent, PluginEnabledEvent, ServiceProvidedEvent, ServiceRemovedEvent,
};
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::ban_service::{BanEntry, BanSource};
use infrarust_api::types::{Component, GameProfile, PlayerId, ServerId};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::error::{HarnessError, HarnessResult};

pub const RECORDER_PLUGIN_ID: &str = "harness_recorder";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    PreLogin,
    GameProfileRequest,
    Login,
    PostLogin,
    PermissionsSetup,
    OnlineAuthFailed,
    Disconnect,
    PlayerChooseInitialServer,
    ServerPreConnect,
    ServerConnected,
    ServerPostConnect,
    KickedFromServer,
    ChatMessage,
    CommandExecute,
    ProxyPing,
    ProxyInitialize,
    ProxyShutdown,
    ConfigReload,
    BackendHealth,
    ServerStateChange,
    BanIssued,
    BanRevoked,
    ConnectionHandshake,
    ConnectionRejected,
    LimboEnter,
    LimboExit,
    PluginEnabled,
    PluginDisabled,
    ServiceProvided,
    ServiceRemoved,
}

impl EventKind {
    pub const ALL: [Self; 30] = [
        Self::PreLogin,
        Self::GameProfileRequest,
        Self::Login,
        Self::PostLogin,
        Self::PermissionsSetup,
        Self::OnlineAuthFailed,
        Self::Disconnect,
        Self::PlayerChooseInitialServer,
        Self::ServerPreConnect,
        Self::ServerConnected,
        Self::ServerPostConnect,
        Self::KickedFromServer,
        Self::ChatMessage,
        Self::CommandExecute,
        Self::ProxyPing,
        Self::ProxyInitialize,
        Self::ProxyShutdown,
        Self::ConfigReload,
        Self::BackendHealth,
        Self::ServerStateChange,
        Self::BanIssued,
        Self::BanRevoked,
        Self::ConnectionHandshake,
        Self::ConnectionRejected,
        Self::LimboEnter,
        Self::LimboExit,
        Self::PluginEnabled,
        Self::PluginDisabled,
        Self::ServiceProvided,
        Self::ServiceRemoved,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::PreLogin => "PreLogin",
            Self::GameProfileRequest => "GameProfileRequest",
            Self::Login => "Login",
            Self::PostLogin => "PostLogin",
            Self::PermissionsSetup => "PermissionsSetup",
            Self::OnlineAuthFailed => "OnlineAuthFailed",
            Self::Disconnect => "Disconnect",
            Self::PlayerChooseInitialServer => "PlayerChooseInitialServer",
            Self::ServerPreConnect => "ServerPreConnect",
            Self::ServerConnected => "ServerConnected",
            Self::ServerPostConnect => "ServerPostConnect",
            Self::KickedFromServer => "KickedFromServer",
            Self::ChatMessage => "ChatMessage",
            Self::CommandExecute => "CommandExecute",
            Self::ProxyPing => "ProxyPing",
            Self::ProxyInitialize => "ProxyInitialize",
            Self::ProxyShutdown => "ProxyShutdown",
            Self::ConfigReload => "ConfigReload",
            Self::BackendHealth => "BackendHealth",
            Self::ServerStateChange => "ServerStateChange",
            Self::BanIssued => "BanIssued",
            Self::BanRevoked => "BanRevoked",
            Self::ConnectionHandshake => "ConnectionHandshake",
            Self::ConnectionRejected => "ConnectionRejected",
            Self::LimboEnter => "LimboEnter",
            Self::LimboExit => "LimboExit",
            Self::PluginEnabled => "PluginEnabled",
            Self::PluginDisabled => "PluginDisabled",
            Self::ServiceProvided => "ServiceProvided",
            Self::ServiceRemoved => "ServiceRemoved",
        }
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub seq: u64,
    pub kind: EventKind,
    pub player: Option<PlayerId>,
    pub username: Option<String>,
    pub detail: Value,
}

#[derive(Debug, Default)]
struct Log {
    events: Vec<Recorded>,
    usernames: HashMap<PlayerId, String>,
}

impl Log {
    fn learn(&mut self, player: PlayerId, username: &str) {
        if username.is_empty() || self.usernames.contains_key(&player) {
            return;
        }
        self.usernames.insert(player, username.to_string());
        for event in &mut self.events {
            if event.player == Some(player) && event.username.is_none() {
                event.username = Some(username.to_string());
            }
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    log: Mutex<Log>,
    changed: Notify,
}

#[derive(Debug, Clone, Default)]
pub struct Recorder {
    inner: Arc<Inner>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plugin(&self) -> RecordingPlugin {
        RecordingPlugin {
            id: RECORDER_PLUGIN_ID.to_string(),
            recorder: self.clone(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Log> {
        self.inner
            .log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn record(
        &self,
        kind: EventKind,
        player: Option<PlayerId>,
        username: Option<&str>,
        detail: Value,
    ) {
        {
            let mut log = self.lock();
            if let (Some(player), Some(username)) = (player, username) {
                log.learn(player, username);
            }
            let username = username
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .or_else(|| player.and_then(|p| log.usernames.get(&p).cloned()));
            let seq = log.events.len() as u64;
            log.events.push(Recorded {
                seq,
                kind,
                player,
                username,
                detail,
            });
        }
        self.inner.changed.notify_waiters();
    }

    pub fn events(&self) -> Vec<Recorded> {
        self.lock().events.clone()
    }

    pub fn kinds(&self) -> Vec<EventKind> {
        self.lock().events.iter().map(|e| e.kind).collect()
    }

    pub fn of(&self, kind: EventKind) -> Vec<Recorded> {
        self.filter(|e| e.kind == kind)
    }

    pub fn for_player(&self, player: PlayerId) -> Vec<Recorded> {
        self.filter(|e| e.player == Some(player))
    }

    pub fn for_username(&self, username: &str) -> Vec<Recorded> {
        self.filter(|e| e.username.as_deref() == Some(username))
    }

    pub fn player_id(&self, username: &str) -> Option<PlayerId> {
        self.lock()
            .usernames
            .iter()
            .find(|(_, name)| name.as_str() == username)
            .map(|(id, _)| *id)
    }

    pub fn count(&self, kind: EventKind) -> usize {
        self.lock().events.iter().filter(|e| e.kind == kind).count()
    }

    pub fn filter(&self, mut pick: impl FnMut(&Recorded) -> bool) -> Vec<Recorded> {
        let mut events = self.events();
        events.retain(|e| pick(e));
        events
    }

    pub async fn wait_for(
        &self,
        mut pick: impl FnMut(&Recorded) -> bool,
        timeout: Duration,
    ) -> HarnessResult<Recorded> {
        let deadline = Instant::now() + timeout;
        loop {
            let changed = self.inner.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(found) = self.events().into_iter().find(|e| pick(e)) {
                return Ok(found);
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                let seen: Vec<String> = self
                    .events()
                    .iter()
                    .map(|e| format!("{}({})", e.kind, e.username.as_deref().unwrap_or("-")))
                    .collect();
                return Err(HarnessError::timeout(
                    format!(
                        "a matching recorded event (seen so far: [{}])",
                        seen.join(", ")
                    ),
                    timeout,
                ));
            }
        }
    }

    pub async fn wait_for_kind(
        &self,
        kind: EventKind,
        timeout: Duration,
    ) -> HarnessResult<Recorded> {
        self.wait_for(|e| e.kind == kind, timeout).await
    }
}

pub struct RecordingPlugin {
    id: String,
    recorder: Recorder,
}

impl RecordingPlugin {
    pub fn new() -> Self {
        Recorder::new().plugin()
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn recorder(&self) -> Recorder {
        self.recorder.clone()
    }
}

impl Default for RecordingPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for RecordingPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new(self.id.clone(), "Harness event recorder", "0.0.0")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        subscribe_all(ctx.event_bus(), &self.recorder);
        subscribe_plugin_events(ctx.event_bus(), &self.recorder);
        Box::pin(async { Ok(()) })
    }
}

type Capture = (EventKind, Option<PlayerId>, Option<String>, Value);

fn on<E: Event>(bus: &dyn EventBus, recorder: &Recorder, capture: fn(&E) -> Capture) {
    let recorder = recorder.clone();
    bus.subscribe(EventPriority::LAST, move |event: &mut E| {
        let (kind, player, username, detail) = capture(event);
        recorder.record(kind, player, username.as_deref(), detail);
    });
}

fn subscribe_all(bus: &dyn EventBus, recorder: &Recorder) {
    on::<PreLoginEvent>(bus, recorder, |e| {
        (
            EventKind::PreLogin,
            None,
            Some(e.profile.username.clone()),
            json!({
                "profile": profile(&e.profile),
                "remote_addr": e.remote_addr.to_string(),
                "protocol_version": e.protocol_version.raw(),
                "server_domain": e.server_domain,
                "result": pre_login_result(e.result()),
            }),
        )
    });
    on::<GameProfileRequestEvent>(bus, recorder, |e| {
        (
            EventKind::GameProfileRequest,
            None,
            Some(e.profile.username.clone()),
            json!({
                "original": profile(e.original()),
                "profile": profile(&e.profile),
                "online_mode": e.online_mode,
                "remote_addr": e.remote_addr.to_string(),
                "virtual_host": e.virtual_host,
                "protocol_version": e.protocol_version.raw(),
            }),
        )
    });
    on::<PostLoginEvent>(bus, recorder, |e| {
        (
            EventKind::PostLogin,
            Some(e.player_id()),
            Some(e.profile.username.clone()),
            json!({
                "profile": profile(&e.profile),
                "protocol_version": e.protocol_version.raw(),
                "remote_addr": e.player.remote_addr().to_string(),
                "current_server": e.player.current_server().as_ref().map(ServerId::as_str),
            }),
        )
    });
    on::<PermissionsSetupEvent>(bus, recorder, |e| {
        let result = match e.result() {
            PermissionsSetupResult::UseDefault => "use_default",
            PermissionsSetupResult::Custom(_) => "custom",
            _ => "other",
        };
        (
            EventKind::PermissionsSetup,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "profile": profile(e.profile()),
                "online_mode": e.online_mode,
                "result": result,
            }),
        )
    });
    on::<LoginEvent>(bus, recorder, |e| {
        let result = match e.result() {
            LoginResult::Allowed => json!("allowed"),
            LoginResult::Denied { reason } => json!({ "denied": reason.to_string() }),
            _ => json!("other"),
        };
        (
            EventKind::Login,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "profile": profile(e.profile()),
                "online_mode": e.online_mode,
                "result": result,
            }),
        )
    });
    on::<OnlineAuthFailed>(bus, recorder, |e| {
        (
            EventKind::OnlineAuthFailed,
            None,
            Some(e.username.clone()),
            json!({ "username": e.username }),
        )
    });
    on::<DisconnectEvent>(bus, recorder, |e| {
        (
            EventKind::Disconnect,
            Some(e.player_id()),
            Some(e.username().to_string()),
            json!({
                "username": e.username(),
                "last_server": e.last_server.as_ref().map(ServerId::as_str),
                "cause": e.cause.as_str(),
                "reason": e.cause.reason().map(ToString::to_string),
                "reason_json": e.cause.reason().map(component_value),
            }),
        )
    });
    on::<PlayerChooseInitialServerEvent>(bus, recorder, |e| {
        let result = match e.result() {
            PlayerChooseInitialServerResult::Allowed => json!("allowed"),
            PlayerChooseInitialServerResult::Redirect(id) => json!({ "redirect": id.as_str() }),
            PlayerChooseInitialServerResult::SendToLimbo { limbo_handlers } => {
                json!({ "send_to_limbo": limbo_handlers })
            }
            _ => json!("other"),
        };
        (
            EventKind::PlayerChooseInitialServer,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "profile": profile(e.profile()),
                "initial_server": e.initial_server.as_str(),
                "current_server": server(e.player.current_server().as_ref()),
                "result": result,
            }),
        )
    });
    on::<ServerPreConnectEvent>(bus, recorder, |e| {
        let result = match e.result() {
            ServerPreConnectResult::Allowed => json!("allowed"),
            ServerPreConnectResult::ConnectTo(id) => json!({ "connect_to": id.as_str() }),
            ServerPreConnectResult::SendToLimbo { limbo_handlers } => {
                json!({ "send_to_limbo": limbo_handlers })
            }
            ServerPreConnectResult::Denied { reason } => {
                json!({ "denied": reason.to_string() })
            }
            _ => json!("other"),
        };
        (
            EventKind::ServerPreConnect,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "profile": profile(e.profile()),
                "server": e.server.as_str(),
                "previous_server": server(e.previous_server.as_ref()),
                "cause": e.cause.as_str(),
                "current_server": server(e.player.current_server().as_ref()),
                "result": result,
            }),
        )
    });
    on::<ServerConnectedEvent>(bus, recorder, |e| {
        (
            EventKind::ServerConnected,
            Some(e.player_id()),
            Some(e.player.profile().username.clone()),
            json!({
                "server": e.server.as_str(),
                "previous_server": server(e.previous_server.as_ref()),
                "current_server": server(e.player.current_server().as_ref()),
            }),
        )
    });
    on::<ServerPostConnectEvent>(bus, recorder, |e| {
        (
            EventKind::ServerPostConnect,
            Some(e.player_id()),
            Some(e.player.profile().username.clone()),
            json!({
                "server": e.server.as_str(),
                "previous_server": server(e.previous_server.as_ref()),
                "current_server": server(e.player.current_server().as_ref()),
            }),
        )
    });
    on::<KickedFromServerEvent>(bus, recorder, |e| {
        let result = match e.result() {
            KickedFromServerResult::DisconnectPlayer { reason } => {
                json!({ "disconnect_player": reason.as_ref().map(ToString::to_string) })
            }
            KickedFromServerResult::RedirectTo(id) => json!({ "redirect_to": id.as_str() }),
            KickedFromServerResult::SendToLimbo { limbo_handlers } => {
                json!({ "send_to_limbo": limbo_handlers })
            }
            KickedFromServerResult::Notify { message } => {
                json!({ "notify": message.to_string() })
            }
            _ => json!("other"),
        };
        (
            EventKind::KickedFromServer,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "server": e.server.as_str(),
                "reason": e.reason.as_ref().map(component_value),
                "cause": e.cause.as_str(),
                "during_connect": e.during_connect,
                "previous_server": server(e.previous_server.as_ref()),
                "current_server": server(e.player.current_server().as_ref()),
                "result": result,
            }),
        )
    });
    on::<ChatMessageEvent>(bus, recorder, |e| {
        let result = match e.result() {
            ChatMessageResult::Allow => json!("allow"),
            ChatMessageResult::Deny { reason } => {
                json!({ "deny": reason.as_ref().map(ToString::to_string) })
            }
            ChatMessageResult::Modify { message } => json!({ "modify": message }),
            _ => json!("other"),
        };
        (
            EventKind::ChatMessage,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "message": e.message,
                "signed": e.signed,
                "server": server(e.server.as_ref()),
                "result": result,
            }),
        )
    });
    on::<CommandExecuteEvent>(bus, recorder, |e| {
        let result = match e.result() {
            CommandExecuteResult::Allow => json!("allow"),
            CommandExecuteResult::Deny { reason } => {
                json!({ "deny": reason.as_ref().map(ToString::to_string) })
            }
            CommandExecuteResult::Modify { command } => json!({ "modify": command }),
            CommandExecuteResult::ForwardToBackend => json!("forward_to_backend"),
            _ => json!("other"),
        };
        (
            EventKind::CommandExecute,
            Some(e.player_id()),
            Some(e.profile().username.clone()),
            json!({
                "command": e.command,
                "signed": e.signed,
                "server": server(e.server.as_ref()),
                "result": result,
            }),
        )
    });
    on::<ProxyPingEvent>(bus, recorder, |e| {
        let sample: Vec<Value> = e
            .response
            .player_sample
            .iter()
            .map(|(name, id)| json!([name, id.to_string()]))
            .collect();
        (
            EventKind::ProxyPing,
            None,
            None,
            json!({
                "remote_addr": e.remote_addr.to_string(),
                "server": server(e.server.as_ref()),
                "virtual_host": e.virtual_host,
                "protocol_version": e.protocol_version.raw(),
                "legacy": e.legacy,
                "description": e.response.description.to_string(),
                "max_players": e.response.max_players,
                "online_players": e.response.online_players,
                "response_protocol_version": e.response.protocol_version.raw(),
                "version_name": e.response.version_name,
                "has_favicon": e.response.favicon.is_some(),
                "player_sample": sample,
            }),
        )
    });
    on::<ProxyInitializeEvent>(bus, recorder, |_| {
        (EventKind::ProxyInitialize, None, None, json!({}))
    });
    on::<ProxyShutdownEvent>(bus, recorder, |_| {
        (EventKind::ProxyShutdown, None, None, json!({}))
    });
    on::<ConfigReloadEvent>(bus, recorder, |e| {
        (
            EventKind::ConfigReload,
            None,
            None,
            json!({
                "provider": e.provider,
                "added": servers(&e.added),
                "removed": servers(&e.removed),
                "updated": servers(&e.updated),
            }),
        )
    });
    on::<BackendHealthEvent>(bus, recorder, |e| {
        let servers: Vec<&str> = e.servers.iter().map(ServerId::as_str).collect();
        (
            EventKind::BackendHealth,
            None,
            None,
            json!({
                "address": e.address.to_string(),
                "servers": servers,
                "state": e.state.as_str(),
            }),
        )
    });
    on::<ServerStateChangeEvent>(bus, recorder, |e| {
        (
            EventKind::ServerStateChange,
            None,
            None,
            json!({
                "server": e.server.as_str(),
                "old_state": format!("{:?}", e.old_state).to_lowercase(),
                "new_state": format!("{:?}", e.new_state).to_lowercase(),
            }),
        )
    });
    on::<BanIssuedEvent>(bus, recorder, |e| {
        (
            EventKind::BanIssued,
            None,
            None,
            ban_detail(&e.entry, &e.source, e.silent),
        )
    });
    on::<BanRevokedEvent>(bus, recorder, |e| {
        (
            EventKind::BanRevoked,
            None,
            None,
            ban_detail(&e.entry, &e.source, e.silent),
        )
    });
    on::<ConnectionHandshakeEvent>(bus, recorder, |e| {
        let result = match e.result() {
            ConnectionHandshakeResult::Allow => json!("allow"),
            ConnectionHandshakeResult::Deny { reason } => {
                json!({ "deny": reason.as_ref().map(ToString::to_string) })
            }
            ConnectionHandshakeResult::DropSilently => json!("drop_silently"),
            _ => json!("other"),
        };
        (
            EventKind::ConnectionHandshake,
            None,
            None,
            json!({
                "remote_addr": e.remote_addr.to_string(),
                "virtual_host": e.virtual_host,
                "raw_host": e.raw_host,
                "port": e.port,
                "protocol_version": e.protocol_version.raw(),
                "intent": e.intent.as_str(),
                "legacy": e.legacy,
                "server": server(e.server.as_ref()),
                "result": result,
            }),
        )
    });
    on::<ConnectionRejectedEvent>(bus, recorder, |e| {
        let plugin = match &e.reason {
            RejectReason::Plugin { plugin_id } => json!(plugin_id),
            _ => Value::Null,
        };
        (
            EventKind::ConnectionRejected,
            None,
            None,
            json!({
                "remote_addr": e.remote_addr.to_string(),
                "virtual_host": e.virtual_host,
                "reason": e.reason.as_str(),
                "plugin": plugin,
            }),
        )
    });
    on::<LimboEnterEvent>(bus, recorder, |e| {
        (
            EventKind::LimboEnter,
            Some(e.player_id()),
            Some(e.player.profile().username.clone()),
            json!({
                "handlers": e.handlers,
                "context": format!("{:?}", e.context),
                "current_server": server(e.player.current_server().as_ref()),
            }),
        )
    });
    on::<LimboExitEvent>(bus, recorder, |e| {
        let reason = match &e.reason {
            LimboExitReason::Kicked { reason } => Some(reason.to_string()),
            _ => None,
        };
        let handlers = match &e.reason {
            LimboExitReason::SentToLimbo { handlers } => json!(handlers),
            _ => Value::Null,
        };
        (
            EventKind::LimboExit,
            Some(e.player_id()),
            Some(e.player.profile().username.clone()),
            json!({
                "reason": e.reason.as_str(),
                "kick_reason": reason,
                "handlers": handlers,
                "next_server": server(e.next_server.as_ref()),
            }),
        )
    });
}

fn subscribe_plugin_events(bus: &dyn EventBus, recorder: &Recorder) {
    on::<PluginEnabledEvent>(bus, recorder, |e| {
        (
            EventKind::PluginEnabled,
            None,
            None,
            json!({ "plugin": e.plugin_id, "version": e.version }),
        )
    });
    on::<PluginDisabledEvent>(bus, recorder, |e| {
        (
            EventKind::PluginDisabled,
            None,
            None,
            json!({ "plugin": e.plugin_id }),
        )
    });
    on::<ServiceProvidedEvent>(bus, recorder, |e| {
        (
            EventKind::ServiceProvided,
            None,
            None,
            json!({ "service": e.service, "provider": e.provider }),
        )
    });
    on::<ServiceRemovedEvent>(bus, recorder, |e| {
        (
            EventKind::ServiceRemoved,
            None,
            None,
            json!({ "service": e.service, "provider": e.provider }),
        )
    });
}

fn ban_detail(entry: &BanEntry, source: &BanSource, silent: bool) -> Value {
    json!({
        "id": entry.id,
        "target": entry.target.to_string(),
        "reason": entry.reason,
        "entry_source": entry.source.to_string(),
        "source": source.to_string(),
        "silent": silent,
    })
}

pub fn component_value(component: &Component) -> Value {
    serde_json::from_str(&component.to_json()).unwrap_or(Value::Null)
}

fn servers(servers: &[ServerId]) -> Value {
    json!(servers.iter().map(ServerId::as_str).collect::<Vec<_>>())
}

fn server(server: Option<&ServerId>) -> Value {
    server.map_or(Value::Null, |server| json!(server.as_str()))
}

fn profile(profile: &GameProfile) -> Value {
    json!({
        "uuid": profile.uuid.to_string(),
        "username": profile.username,
        "properties": profile.properties.len(),
    })
}

fn pre_login_result(result: &PreLoginResult) -> Value {
    match result {
        PreLoginResult::Allowed => json!("allowed"),
        PreLoginResult::Denied { reason } => json!({ "denied": reason.to_string() }),
        PreLoginResult::ForceOffline => json!("force_offline"),
        PreLoginResult::ForceOnline => json!("force_online"),
        _ => json!("other"),
    }
}
