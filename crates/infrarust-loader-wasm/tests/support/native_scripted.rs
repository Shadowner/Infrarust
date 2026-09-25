use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::error::PluginError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{BoxFuture, EventPriority, ResultedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::connection::{
    KickedFromServerEvent, KickedFromServerResult, PlayerChooseInitialServerEvent,
    PlayerChooseInitialServerResult, ServerConnectedEvent, ServerPostConnectEvent,
    ServerPreConnectEvent, ServerPreConnectResult,
};
use infrarust_api::events::lifecycle::{
    DisconnectEvent, OnlineAuthFailed, PermissionsSetupEvent, PermissionsSetupResult,
    PostLoginEvent, PreLoginEvent, PreLoginResult,
};
use infrarust_api::events::proxy::{
    ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent, ProxyShutdownEvent,
    ServerStateChangeEvent,
};
use infrarust_api::permissions::{PermissionChecker, Tristate};
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::{Component, PlayerId, ServerId};

use super::script::{self, Action, Directive, EventName};

pub struct ScriptedPlugin {
    id: String,
    log: Mutex<Option<PathBuf>>,
}

impl ScriptedPlugin {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            log: Mutex::new(None),
        }
    }
}

impl Plugin for ScriptedPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new(self.id.clone(), "Scripted Native", "0.0.0")
    }

    fn on_enable<'a>(
        &'a self,
        ctx: &'a dyn PluginContext,
    ) -> BoxFuture<'a, Result<(), PluginError>> {
        Box::pin(async move {
            let data_dir = ctx.data_dir();
            let source = std::fs::read_to_string(data_dir.join(script::SCRIPT_FILE))
                .map_err(|e| PluginError::InitFailed(format!("reading the script: {e}")))?;
            let log = data_dir.join(script::LOG_FILE);
            for directive in script::parse(&source).map_err(PluginError::InitFailed)? {
                match directive {
                    Directive::On {
                        event,
                        priority,
                        action,
                    } => subscribe(ctx.event_bus(), log.clone(), event, priority, action),
                    Directive::Cmd { name } => {
                        let spec = CommandSpec::new(name.as_str());
                        let command = Box::new(ScriptedCommand {
                            name,
                            log: log.clone(),
                        });
                        let _ = ctx.command_manager().register(spec, command);
                    }
                }
            }
            script::append(&log, "enable");
            *self.log.lock().expect("log lock") = Some(log);
            Ok(())
        })
    }

    fn on_disable(&self) -> BoxFuture<'_, Result<(), PluginError>> {
        Box::pin(async move {
            if let Some(log) = self.log.lock().expect("log lock").as_ref() {
                script::append(log, "disable");
            }
            Ok(())
        })
    }
}

struct ScriptedCommand {
    name: String,
    log: PathBuf,
}

impl CommandHandler for ScriptedCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let player = ctx.source.player_id().map(PlayerId::as_u64);
            let line = script::command_line(&self.name, &ctx.args, player);
            script::observe(&self.log, &line, &Action::Record);
        })
    }
}

struct FixedLevel(bool);

impl PermissionChecker for FixedLevel {
    fn value(&self, _permission: &str) -> Tristate {
        Tristate::from_bool(self.0)
    }
}

fn text(message: &str) -> Component {
    Component::text(message)
}

fn state(state: ServerState) -> &'static str {
    match state {
        ServerState::Online => "online",
        ServerState::Offline => "offline",
        ServerState::Starting => "starting",
        ServerState::Stopping => "stopping",
        ServerState::Sleeping => "sleeping",
        ServerState::Crashed => "crashed",
        _ => "unknown",
    }
}

struct Seen {
    log: PathBuf,
    event: EventName,
    priority: u8,
    action: Action,
}

impl Seen {
    fn record(&self, fields: &[&str]) {
        let line = script::event_line(self.event, self.priority, fields);
        script::observe(&self.log, &line, &self.action);
    }
}

fn subscribe(bus: &dyn EventBus, log: PathBuf, event: EventName, priority: u8, action: Action) {
    let cancel = action == Action::Cancelled;
    let at = EventPriority::custom(priority);
    let seen = Seen {
        log,
        event,
        priority,
        action,
    };
    let handle = match event {
        EventName::PreLogin => bus.subscribe(at, move |e: &mut PreLoginEvent| {
            let uuid = e.profile.uuid.to_string();
            let addr = e.remote_addr.to_string();
            let protocol = e.protocol_version.raw().to_string();
            seen.record(&[
                &e.profile.username,
                &uuid,
                &addr,
                &protocol,
                &e.server_domain,
            ]);
            let result = match &seen.action {
                Action::Allow => PreLoginResult::Allowed,
                Action::Deny(reason) => PreLoginResult::Denied {
                    reason: text(reason),
                },
                Action::ForceOffline => PreLoginResult::ForceOffline,
                Action::ForceOnline => PreLoginResult::ForceOnline,
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::PostLogin => bus.subscribe(at, move |e: &mut PostLoginEvent| {
            let id = e.player_id().as_u64().to_string();
            let uuid = e.profile.uuid.to_string();
            let protocol = e.protocol_version.raw().to_string();
            seen.record(&[&id, &e.profile.username, &uuid, &protocol]);
        }),
        EventName::Disconnect => bus.subscribe(at, move |e: &mut DisconnectEvent| {
            let id = e.player_id().as_u64().to_string();
            let last = e.last_server.as_ref().map_or("-", ServerId::as_str);
            seen.record(&[&id, e.username(), last]);
        }),
        EventName::OnlineAuthFailed => bus.subscribe(at, move |e: &mut OnlineAuthFailed| {
            seen.record(&[&e.username]);
        }),
        EventName::PermissionsSetup => bus.subscribe(at, move |e: &mut PermissionsSetupEvent| {
            let id = e.player_id().as_u64().to_string();
            let online = e.online_mode.to_string();
            seen.record(&[&id, &e.profile().username, &online]);
            if let Action::Custom(level) = &seen.action {
                e.set_result(PermissionsSetupResult::Custom(Arc::new(FixedLevel(
                    level == "admin",
                ))));
            }
        }),
        EventName::ServerPreConnect => bus.subscribe(at, move |e: &mut ServerPreConnectEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, &e.profile().username, e.server.as_str()]);
            let result = match &seen.action {
                Action::Allow => ServerPreConnectResult::Allowed,
                Action::ConnectTo(server) => {
                    ServerPreConnectResult::ConnectTo(ServerId::new(server.as_str()))
                }
                Action::Limbo(handlers) => ServerPreConnectResult::SendToLimbo {
                    limbo_handlers: handlers.clone(),
                },
                Action::Deny(reason) => ServerPreConnectResult::Denied {
                    reason: text(reason),
                },
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::ServerConnected => bus.subscribe(at, move |e: &mut ServerConnectedEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, e.server.as_str()]);
        }),
        EventName::ServerSwitch => bus.subscribe(at, move |e: &mut ServerPostConnectEvent| {
            if let Some(previous) = e.switched_from() {
                let id = e.player_id().as_u64().to_string();
                seen.record(&[&id, previous.as_str(), e.server.as_str()]);
            }
        }),
        EventName::KickedFromServer => bus.subscribe(at, move |e: &mut KickedFromServerEvent| {
            let id = e.player_id().as_u64().to_string();
            let reason = e
                .reason
                .as_ref()
                .map_or_else(|| Component::text("").to_json(), Component::to_json);
            seen.record(&[&id, e.server.as_str(), &reason]);
            let result = match &seen.action {
                Action::Redirect(server) => {
                    KickedFromServerResult::RedirectTo(ServerId::new(server.as_str()))
                }
                Action::Limbo(handlers) => KickedFromServerResult::SendToLimbo {
                    limbo_handlers: handlers.clone(),
                },
                Action::Notify(message) => KickedFromServerResult::Notify {
                    message: text(message),
                },
                Action::Disconnect(reason) => KickedFromServerResult::DisconnectPlayer {
                    reason: Some(text(reason)),
                },
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::PlayerChooseInitialServer => {
            bus.subscribe(at, move |e: &mut PlayerChooseInitialServerEvent| {
                let id = e.player_id().as_u64().to_string();
                seen.record(&[&id, &e.profile().username, e.initial_server.as_str()]);
                let result = match &seen.action {
                    Action::Allow => PlayerChooseInitialServerResult::Allowed,
                    Action::Redirect(server) => {
                        PlayerChooseInitialServerResult::Redirect(ServerId::new(server.as_str()))
                    }
                    Action::Limbo(handlers) => PlayerChooseInitialServerResult::SendToLimbo {
                        limbo_handlers: handlers.clone(),
                    },
                    _ => return,
                };
                e.set_result(result);
            })
        }
        EventName::ProxyPing => bus.subscribe(at, move |e: &mut ProxyPingEvent| {
            let addr = e.remote_addr.to_string();
            let response = &e.response;
            let description = response.description.to_json();
            let max = response.max_players.to_string();
            let online = response.online_players.to_string();
            let protocol = response.protocol_version.raw().to_string();
            let favicon = response.favicon.as_deref().unwrap_or("-");
            seen.record(&[
                &addr,
                &description,
                &max,
                &online,
                &protocol,
                &response.version_name,
                favicon,
            ]);
            if let Action::Description(description) = &seen.action {
                e.response.description = text(description);
            }
        }),
        EventName::ProxyInitialize => {
            bus.subscribe(at, move |_: &mut ProxyInitializeEvent| seen.record(&[]))
        }
        EventName::ProxyShutdown => {
            bus.subscribe(at, move |_: &mut ProxyShutdownEvent| seen.record(&[]))
        }
        EventName::ConfigReload => {
            bus.subscribe(at, move |_: &mut ConfigReloadEvent| seen.record(&[]))
        }
        EventName::ServerStateChange => bus.subscribe(at, move |e: &mut ServerStateChangeEvent| {
            seen.record(&[e.server.as_str(), state(e.old_state), state(e.new_state)]);
        }),
        EventName::ChatMessage => bus.subscribe(at, move |e: &mut ChatMessageEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, &e.message]);
            let result = match &seen.action {
                Action::Allow => ChatMessageResult::Allow,
                Action::Deny(reason) => ChatMessageResult::Deny {
                    reason: Some(text(reason)),
                },
                Action::Modify(message) => ChatMessageResult::Modify {
                    message: message.clone(),
                },
                _ => return,
            };
            e.set_result(result);
        }),
    };
    if cancel {
        bus.unsubscribe(handle);
    }
}
