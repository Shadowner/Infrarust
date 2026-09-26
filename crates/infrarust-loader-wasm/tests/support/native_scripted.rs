use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use infrarust_api::command::{CommandContext, CommandHandler, CommandSpec};
use infrarust_api::error::PluginError;
use infrarust_api::event::bus::{EventBus, EventBusExt};
use infrarust_api::event::{
    BoxFuture, ConnectionState, EventPriority, PacketFilter, ResultedEvent,
};
use infrarust_api::events::ban::{BanIssuedEvent, BanRevokedEvent};
use infrarust_api::events::chat::{ChatMessageEvent, ChatMessageResult};
use infrarust_api::events::client::{
    PlayerChannelRegisterEvent, PlayerClientBrandEvent, PlayerSettingsChangedEvent,
};
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
use infrarust_api::events::limbo::{LimboEnterEvent, LimboExitEvent};
use infrarust_api::events::messaging::{PluginMessageEvent, PluginMessageResult};
use infrarust_api::events::named::NamedEvent;
use infrarust_api::events::packet::{PacketDirection, RawPacketEvent, RawPacketResult};
use infrarust_api::events::plugin::{PluginDisabledEvent, PluginEnabledEvent};
use infrarust_api::events::proxy::{
    BackendHealthEvent, ConfigReloadEvent, ProxyInitializeEvent, ProxyPingEvent,
    ProxyShutdownEvent, ServerStateChangeEvent,
};
use infrarust_api::events::resource_pack::PlayerResourcePackStatusEvent;
use infrarust_api::events::transfer::{PreTransferEvent, PreTransferResult};
use infrarust_api::limbo::context::LimboEntryContext;
use infrarust_api::messaging::{ChannelId, Endpoint, MessagePhase};
use infrarust_api::permissions::{PermissionChecker, Tristate};
use infrarust_api::player::MainHand;
use infrarust_api::plugin::{Plugin, PluginContext, PluginMetadata};
use infrarust_api::services::ban_service::{BanEntry, BanSource, BanTarget};
use infrarust_api::services::server_manager::ServerState;
use infrarust_api::types::RawPacket;
use infrarust_api::types::{Component, PlayerId, ServerId};

use super::script::{self, Action, Directive, EventName, joined, or_dash, text as bytes_text};

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
                    Directive::Fire {
                        command,
                        event,
                        payload,
                    } => {
                        let spec = CommandSpec::new(command.as_str());
                        let fire = Box::new(FireCommand {
                            command,
                            event,
                            payload,
                            bus: ctx.event_bus_handle(),
                            log: log.clone(),
                        });
                        let _ = ctx.command_manager().register(spec, fire);
                    }
                    Directive::Named {
                        name,
                        priority,
                        action,
                    } => subscribe_named(ctx.event_bus(), log.clone(), name, priority, action),
                    Directive::Channel { id } => {
                        let channel = ChannelId::modern(&id)
                            .map_err(|e| PluginError::InitFailed(e.to_string()))?;
                        ctx.channel_registrar().register(channel);
                    }
                    Directive::Config { key } => {
                        let value = ctx.config_service().get_value(&key);
                        script::append(&log, &script::config_line(&key, value.as_deref()));
                    }
                    Directive::Plugin { id } => {
                        let state = ctx.plugin_registry().plugin_info(&id).map(|p| p.state);
                        script::append(&log, &script::plugin_line(&id, state.as_deref()));
                    }
                    Directive::Connect { command, server } => {
                        let spec = CommandSpec::new(command.as_str());
                        let connect = Box::new(ConnectCommand {
                            command,
                            server,
                            log: log.clone(),
                        });
                        let _ = ctx.command_manager().register(spec, connect);
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

struct ConnectCommand {
    command: String,
    server: String,
    log: PathBuf,
}

impl CommandHandler for ConnectCommand {
    fn execute<'a>(&'a self, ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let outcome = match ctx.source.player() {
                Some(player) => match player.connect(ServerId::new(self.server.as_str())).await {
                    Ok(result) => result.as_str().to_owned(),
                    Err(error) => format!("{error:?}"),
                },
                None => "console".to_owned(),
            };
            let origin = format!("cmd {}", self.command);
            script::append(
                &self.log,
                &script::connect_line(&origin, &self.server, &outcome),
            );
            ctx.source
                .send_message(Component::text(format!("{} {outcome}", self.command)));
        })
    }
}

struct FireCommand {
    command: String,
    event: String,
    payload: String,
    bus: Arc<dyn EventBus>,
    log: PathBuf,
}

impl CommandHandler for FireCommand {
    fn execute<'a>(&'a self, _ctx: CommandContext) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let event = NamedEvent::new(self.event.as_str(), "text/plain", self.payload.clone());
            let line = match self.bus.fire(event).await {
                Ok(event) => script::fired_line(
                    &self.command,
                    &self.event,
                    event.cancelled,
                    event
                        .response
                        .as_ref()
                        .and_then(|response| std::str::from_utf8(&response.payload).ok()),
                ),
                Err(error) => format!("cmd {} failed {error}", self.command),
            };
            script::append(&self.log, &line);
        })
    }
}

fn named_fields(e: &NamedEvent) -> Vec<String> {
    vec![
        or_dash(Some(e.source_plugin.as_str()).filter(|s| !s.is_empty())).to_owned(),
        e.content_type.clone(),
        bytes_text(&e.payload),
        e.cancelled.to_string(),
        or_dash(
            e.response
                .as_ref()
                .and_then(|response| std::str::from_utf8(&response.payload).ok()),
        )
        .to_owned(),
    ]
}

fn answer_named(e: &mut NamedEvent, action: &Action) {
    match action {
        Action::Cancel => e.cancel(),
        Action::Respond(text) => e.respond("text/plain", text.clone()),
        _ => {}
    }
}

fn subscribe_named(bus: &dyn EventBus, log: PathBuf, name: String, priority: u8, action: Action) {
    let cancel = action == Action::Cancelled;
    let handle = bus.subscribe(
        EventPriority::custom(priority),
        move |e: &mut NamedEvent| {
            if e.name != name {
                return;
            }
            let fields = named_fields(e);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            script::observe(&log, &script::named_line(&name, priority, &fields), &action);
            answer_named(e, &action);
        },
    );
    if cancel {
        bus.unsubscribe(handle);
    }
}

fn direction(direction: PacketDirection) -> &'static str {
    match direction {
        PacketDirection::Clientbound => "clientbound",
        _ => "serverbound",
    }
}

fn ban_target(target: &BanTarget) -> String {
    match target {
        BanTarget::Username(name) => format!("username:{name}"),
        _ => "other".to_owned(),
    }
}

fn ban_fields(entry: &BanEntry, source: &BanSource, silent: bool) -> Vec<String> {
    vec![
        entry.id.clone(),
        ban_target(&entry.target),
        or_dash(entry.reason.as_deref()).to_owned(),
        source.to_string(),
        silent.to_string(),
    ]
}

fn reject_reason(reason: &RejectReason) -> String {
    match reason {
        RejectReason::Plugin { plugin_id } => format!("plugin:{}", or_dash(plugin_id.as_deref())),
        other => other.as_str().to_owned(),
    }
}

fn limbo_context(context: &LimboEntryContext) -> String {
    match context {
        LimboEntryContext::InitialConnection { target_server } => {
            format!("initial:{target_server}")
        }
        LimboEntryContext::KickedFromServer { server, .. } => format!("kicked:{server}"),
        LimboEntryContext::PluginRedirect { from_server } => format!(
            "redirect:{}",
            or_dash(from_server.as_ref().map(ServerId::as_str))
        ),
        _ => "other".to_owned(),
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

fn previous(server: Option<&ServerId>) -> &str {
    or_dash(server.map(ServerId::as_str))
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
        EventName::Login => bus.subscribe(at, move |e: &mut LoginEvent| {
            let id = e.player_id().as_u64().to_string();
            let online = e.online_mode.to_string();
            seen.record(&[&id, &e.profile().username, &online]);
            let result = match &seen.action {
                Action::Allow => LoginResult::Allowed,
                Action::Deny(reason) => LoginResult::Denied {
                    reason: text(reason),
                },
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::GameProfileRequest => {
            bus.subscribe(at, move |e: &mut GameProfileRequestEvent| {
                let original = e.original().clone();
                let uuid = original.uuid.to_string();
                let online = e.online_mode.to_string();
                let remote = e.remote_addr.to_string();
                let protocol = e.protocol_version.raw().to_string();
                seen.record(&[
                    &original.username,
                    &uuid,
                    &online,
                    &remote,
                    or_dash(e.virtual_host.as_deref()),
                    &protocol,
                    &e.profile.username,
                ]);
                if let Action::Rename(name) = &seen.action {
                    e.profile.username = name.clone();
                }
            })
        }
        EventName::CommandExecute => bus.subscribe(at, move |e: &mut CommandExecuteEvent| {
            let id = e.player_id().as_u64().to_string();
            let signed = e.signed.to_string();
            seen.record(&[&id, &e.command, &signed, previous(e.server.as_ref())]);
            let result = match &seen.action {
                Action::Allow => CommandExecuteResult::Allow,
                Action::Deny(reason) => CommandExecuteResult::Deny {
                    reason: Some(text(reason)),
                },
                Action::Modify(command) => CommandExecuteResult::Modify {
                    command: command.clone(),
                },
                Action::ForwardToBackend => CommandExecuteResult::ForwardToBackend,
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::ConnectionHandshake => {
            bus.subscribe(at, move |e: &mut ConnectionHandshakeEvent| {
                let remote = e.remote_addr.to_string();
                let port = e.port.to_string();
                let protocol = e.protocol_version.raw().to_string();
                let legacy = e.legacy.to_string();
                seen.record(&[
                    &remote,
                    or_dash(e.virtual_host.as_deref()),
                    &e.raw_host,
                    &port,
                    &protocol,
                    e.intent.as_str(),
                    &legacy,
                    previous(e.server.as_ref()),
                ]);
                let result = match &seen.action {
                    Action::Allow => ConnectionHandshakeResult::Allow,
                    Action::Deny(reason) => ConnectionHandshakeResult::Deny {
                        reason: Some(text(reason)),
                    },
                    Action::Drop => ConnectionHandshakeResult::DropSilently,
                    _ => return,
                };
                e.set_result(result);
            })
        }
        EventName::ConnectionRejected => {
            bus.subscribe(at, move |e: &mut ConnectionRejectedEvent| {
                let remote = e.remote_addr.to_string();
                seen.record(&[
                    &remote,
                    or_dash(e.virtual_host.as_deref()),
                    &reject_reason(&e.reason),
                ]);
            })
        }
        EventName::LimboEnter => bus.subscribe(at, move |e: &mut LimboEnterEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, &joined(e.handlers.iter()), &limbo_context(&e.context)]);
        }),
        EventName::LimboExit => bus.subscribe(at, move |e: &mut LimboExitEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, e.reason.as_str(), previous(e.next_server.as_ref())]);
        }),
        EventName::PlayerClientBrand => bus.subscribe(at, move |e: &mut PlayerClientBrandEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, &e.brand]);
        }),
        EventName::PlayerSettingsChanged => {
            bus.subscribe(at, move |e: &mut PlayerSettingsChangedEvent| {
                let id = e.player_id().as_u64().to_string();
                let view = e.settings.view_distance.to_string();
                let hand = match e.settings.main_hand {
                    MainHand::Left => "left",
                    _ => "right",
                };
                seen.record(&[&id, &e.settings.locale, &view, hand]);
            })
        }
        EventName::PlayerChannelRegister => {
            bus.subscribe(at, move |e: &mut PlayerChannelRegisterEvent| {
                let id = e.player_id().as_u64().to_string();
                seen.record(&[&id, &joined(e.channels.iter()), direction(e.direction)]);
            })
        }
        EventName::PluginMessage => bus.subscribe(at, move |e: &mut PluginMessageEvent| {
            let id = e.player_id().as_u64().to_string();
            let source = match &e.source {
                Endpoint::Backend(server) => format!("backend:{server}"),
                _ => "client".to_owned(),
            };
            let phase = match e.phase {
                MessagePhase::Configuration => "configuration",
                _ => "play",
            };
            seen.record(&[
                &id,
                &source,
                or_dash(e.channel.modern_id()),
                or_dash(e.channel.legacy_name()),
                &e.raw_channel,
                &bytes_text(&e.data),
                phase,
            ]);
            let result = match &seen.action {
                Action::Forward => PluginMessageResult::Forward,
                Action::Handled => PluginMessageResult::Handled,
                Action::Replace(data) => PluginMessageResult::Replace(Bytes::from(data.clone())),
                Action::Reply(data) => {
                    let _ = e
                        .player
                        .send_plugin_message(&e.channel, Bytes::from(data.clone()));
                    PluginMessageResult::Handled
                }
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::BanIssued => bus.subscribe(at, move |e: &mut BanIssuedEvent| {
            let fields = ban_fields(&e.entry, &e.source, e.silent);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen.record(&fields);
        }),
        EventName::BanRevoked => bus.subscribe(at, move |e: &mut BanRevokedEvent| {
            let fields = ban_fields(&e.entry, &e.source, e.silent);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen.record(&fields);
        }),
        EventName::PluginEnabled => bus.subscribe(at, move |e: &mut PluginEnabledEvent| {
            seen.record(&[&e.plugin_id, &e.version]);
        }),
        EventName::PluginDisabled => bus.subscribe(at, move |e: &mut PluginDisabledEvent| {
            seen.record(&[&e.plugin_id]);
        }),
        EventName::PreTransfer => bus.subscribe(at, move |e: &mut PreTransferEvent| {
            let id = e.player_id().as_u64().to_string();
            let port = e.port.to_string();
            seen.record(&[&id, &e.host, &port, e.origin.as_str()]);
            let result = match &seen.action {
                Action::Allow => PreTransferResult::Allowed,
                Action::Deny(reason) => PreTransferResult::Denied {
                    reason: text(reason),
                },
                Action::Redirect(target) => {
                    let Some((host, port)) = target.rsplit_once(':') else {
                        return;
                    };
                    let Ok(port) = port.parse() else {
                        return;
                    };
                    PreTransferResult::Redirect {
                        host: host.to_owned(),
                        port,
                    }
                }
                _ => return,
            };
            e.set_result(result);
        }),
        EventName::PlayerResourcePackStatus => {
            bus.subscribe(at, move |e: &mut PlayerResourcePackStatusEvent| {
                let id = e.player_id().as_u64().to_string();
                let pack = e
                    .pack_id
                    .map_or_else(|| "-".to_owned(), |pack| pack.to_string());
                seen.record(&[&id, &pack, e.status.as_str(), e.origin.as_str()]);
            })
        }
        EventName::NamedEvent => bus.subscribe(at, move |e: &mut NamedEvent| {
            let mut fields = vec![e.name.clone()];
            fields.extend(named_fields(e));
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen.record(&fields);
            answer_named(e, &seen.action);
        }),
        EventName::RawPacket => bus.subscribe_packet_typed(
            PacketFilter {
                packet_id: script::PACKET_ID,
                state: ConnectionState::Play,
                direction: PacketDirection::Serverbound,
            },
            at,
            move |e: &mut RawPacketEvent| {
                let id = e.player_id.as_u64().to_string();
                let packet = e.packet.packet_id.to_string();
                seen.record(&[
                    &id,
                    direction(e.direction),
                    &packet,
                    &bytes_text(&e.packet.data),
                ]);
                let result = match &seen.action {
                    Action::Pass => RawPacketResult::Pass,
                    Action::Drop => RawPacketResult::Drop,
                    Action::Modify(data) => RawPacketResult::Modify {
                        packet: RawPacket::new(e.packet.packet_id, Bytes::from(data.clone())),
                    },
                    _ => return,
                };
                e.set_result(result);
            },
        ),
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
            seen.record(&[
                &id,
                e.username(),
                previous(e.last_server.as_ref()),
                e.cause.as_str(),
            ]);
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
            seen.record(&[
                &id,
                &e.profile().username,
                e.server.as_str(),
                previous(e.previous_server.as_ref()),
                e.cause.as_str(),
            ]);
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
            seen.record(&[&id, e.server.as_str(), previous(e.previous_server.as_ref())]);
        }),
        EventName::ServerPostConnect => bus.subscribe(at, move |e: &mut ServerPostConnectEvent| {
            let id = e.player_id().as_u64().to_string();
            seen.record(&[&id, e.server.as_str(), previous(e.previous_server.as_ref())]);
        }),
        EventName::KickedFromServer => bus.subscribe(at, move |e: &mut KickedFromServerEvent| {
            let id = e.player_id().as_u64().to_string();
            let reason = e
                .reason
                .as_ref()
                .map_or_else(|| "-".to_owned(), Component::to_json);
            let during = e.during_connect.to_string();
            seen.record(&[
                &id,
                e.server.as_str(),
                &reason,
                e.cause.as_str(),
                &during,
                previous(e.previous_server.as_ref()),
            ]);
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
            let protocol = e.protocol_version.raw().to_string();
            let legacy = e.legacy.to_string();
            let response = &e.response;
            let description = response.description.to_json();
            let max = response.max_players.to_string();
            let online = response.online_players.to_string();
            let answered = response.protocol_version.raw().to_string();
            seen.record(&[
                &addr,
                previous(e.server.as_ref()),
                or_dash(e.virtual_host.as_deref()),
                &protocol,
                &legacy,
                &description,
                &max,
                &online,
                &answered,
                &response.version_name,
                or_dash(response.favicon.as_deref()),
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
        EventName::ConfigReload => bus.subscribe(at, move |e: &mut ConfigReloadEvent| {
            seen.record(&[
                &e.provider,
                &joined(e.added.iter().map(ServerId::as_str)),
                &joined(e.removed.iter().map(ServerId::as_str)),
                &joined(e.updated.iter().map(ServerId::as_str)),
            ]);
        }),
        EventName::ServerStateChange => bus.subscribe(at, move |e: &mut ServerStateChangeEvent| {
            seen.record(&[e.server.as_str(), state(e.old_state), state(e.new_state)]);
        }),
        EventName::ChatMessage => bus.subscribe(at, move |e: &mut ChatMessageEvent| {
            let id = e.player_id().as_u64().to_string();
            let signed = e.signed.to_string();
            seen.record(&[&id, &e.message, &signed, previous(e.server.as_ref())]);
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
        EventName::BackendHealth => bus.subscribe(at, move |e: &mut BackendHealthEvent| {
            let address = format!("{}:{}", e.address.host, e.address.port);
            seen.record(&[
                &address,
                &joined(e.servers.iter().map(ServerId::as_str)),
                e.state.as_str(),
            ]);
        }),
    };
    if cancel {
        bus.unsubscribe(handle);
    }
}
