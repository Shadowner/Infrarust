use std::path::{Path, PathBuf};

use infrarust_plugin_sdk::prelude::*;

use crate::script::{self, Action, Directive, EventName, joined, or_dash, text as bytes_text};

fn log() -> PathBuf {
    Path::new("/").join(script::LOG_FILE)
}

fn text(message: &str) -> Component {
    Component::text(message)
}

fn json(component: &Component) -> String {
    component
        .to_json()
        .unwrap_or_else(|error| format!("<{error}>"))
}

fn seen(event: EventName, priority: u8, fields: &[&str], action: &Action) {
    script::observe(&log(), &script::event_line(event, priority, fields), action);
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

fn disconnect_cause(cause: &DisconnectCause) -> &'static str {
    match cause {
        DisconnectCause::ClientQuit => "client_quit",
        DisconnectCause::Kicked(_) => "kicked",
        DisconnectCause::BackendClosed(_) => "backend_closed",
        DisconnectCause::Shutdown => "shutdown",
        _ => "error",
    }
}

fn connect_cause(cause: ConnectCause) -> &'static str {
    match cause {
        ConnectCause::Initial => "initial",
        ConnectCause::Switch => "switch",
        ConnectCause::LimboExit => "limbo_exit",
        ConnectCause::KickRedirect => "kick_redirect",
        _ => "plugin_message",
    }
}

fn kick_cause(cause: &KickCause) -> &'static str {
    match cause {
        KickCause::Unreachable(_) => "unreachable",
        KickCause::LoginRefused => "login_refused",
        KickCause::ConfigDisconnect => "config_disconnect",
        KickCause::PlayDisconnect => "play_disconnect",
        _ => "connection_lost",
    }
}

fn backend_state(state: BackendState) -> &'static str {
    match state {
        BackendState::Healthy => "healthy",
        BackendState::Probing => "probing",
        BackendState::Unhealthy => "unhealthy",
        _ => "draining",
    }
}

fn direction(direction: PacketDirection) -> &'static str {
    match direction {
        PacketDirection::Clientbound => "clientbound",
        _ => "serverbound",
    }
}

fn channel_fields(channel: &ChannelId) -> [String; 2] {
    [
        or_dash(channel.modern_id()).to_owned(),
        or_dash(channel.legacy_name()).to_owned(),
    ]
}

fn ban_target(target: &BanTarget) -> String {
    match target {
        BanTarget::Username(name) => format!("username:{name}"),
        _ => "other".to_owned(),
    }
}

fn ban_source(source: &BanSource) -> String {
    match source {
        BanSource::Console => "console".to_owned(),
        BanSource::Player { name, .. } => format!("player:{name}"),
        BanSource::Plugin(id) => format!("plugin:{id}"),
        BanSource::WebApi(None) => "web-api".to_owned(),
        BanSource::WebApi(Some(actor)) => format!("web-api:{actor}"),
        _ => "system".to_owned(),
    }
}

fn ban_fields(entry: &BanEntry, source: &BanSource, silent: bool) -> Vec<String> {
    vec![
        entry.id.clone(),
        ban_target(&entry.target),
        or_dash(entry.reason.as_deref()).to_owned(),
        ban_source(source),
        silent.to_string(),
    ]
}

fn pack_status(status: ResourcePackStatus) -> &'static str {
    match status {
        ResourcePackStatus::SuccessfullyLoaded => "successfully_loaded",
        ResourcePackStatus::Declined => "declined",
        ResourcePackStatus::FailedDownload => "failed_download",
        ResourcePackStatus::Accepted => "accepted",
        ResourcePackStatus::Downloaded => "downloaded",
        ResourcePackStatus::InvalidUrl => "invalid_url",
        ResourcePackStatus::FailedReload => "failed_reload",
        ResourcePackStatus::Discarded => "discarded",
        _ => "unknown",
    }
}

fn reject_reason(reason: &RejectReason) -> String {
    match reason {
        RejectReason::IpFilter => "ip_filter".to_owned(),
        RejectReason::RateLimit => "rate_limit".to_owned(),
        RejectReason::UnknownDomain => "unknown_domain".to_owned(),
        RejectReason::IpBanned => "ip_banned".to_owned(),
        RejectReason::Banned => "banned".to_owned(),
        RejectReason::ServerUnavailable => "server_unavailable".to_owned(),
        RejectReason::Plugin(id) => format!("plugin:{}", or_dash(id.as_deref())),
        _ => "other".to_owned(),
    }
}

fn limbo_context(context: &EntryContext) -> String {
    match context {
        EntryContext::InitialConnection(server) => format!("initial:{server}"),
        EntryContext::KickedFromServer { server, .. } => format!("kicked:{server}"),
        EntryContext::PluginRedirect(server) => {
            format!(
                "redirect:{}",
                or_dash(server.as_ref().map(ServerId::as_str))
            )
        }
        _ => "other".to_owned(),
    }
}

fn limbo_exit(reason: &LimboExitReason) -> &'static str {
    match reason {
        LimboExitReason::Released => "released",
        LimboExitReason::Redirected => "redirected",
        LimboExitReason::SentToLimbo(_) => "sent_to_limbo",
        LimboExitReason::Kicked(_) => "kicked",
        LimboExitReason::TimedOut => "timed_out",
        LimboExitReason::Shutdown => "shutdown",
        _ => "disconnected",
    }
}

fn named_fields(e: &NamedEvent) -> Vec<String> {
    vec![
        or_dash(Some(e.source_plugin.as_str()).filter(|s| !s.is_empty())).to_owned(),
        e.content_type.clone(),
        bytes_text(&e.payload),
        e.is_cancelled().to_string(),
        or_dash(e.response().and_then(NamedResponse::text)).to_owned(),
    ]
}

fn answer_named(e: &mut NamedEvent, action: &Action) {
    match action {
        Action::Cancel => e.cancel(),
        Action::Respond(text) => e.respond_text(text.as_str()),
        _ => {}
    }
}

fn subscribe_named(ctx: &Context, name: String, priority: u8, action: Action) {
    let cancel = action == Action::Cancelled;
    let subscription = ctx.on_named(name.clone(), EventPriority::Custom(priority), move |e| {
        let fields = named_fields(e);
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        script::observe(
            &log(),
            &script::named_line(&name, priority, &fields),
            &action,
        );
        answer_named(e, &action);
    });
    if cancel && let Ok(subscription) = subscription {
        subscription.cancel();
    }
}

pub fn enable(ctx: &Context) -> Result<(), PluginError> {
    let source = std::fs::read_to_string(Path::new("/").join(script::SCRIPT_FILE))
        .map_err(|e| format!("reading {}: {e}", script::SCRIPT_FILE))?;
    for directive in script::parse(&source)? {
        match directive {
            Directive::On {
                event,
                priority,
                action,
            } => subscribe(ctx, event, priority, action),
            Directive::Cmd { name } => {
                let label = name.clone();
                let _ = ctx
                    .command(&label)
                    .handler(move |invocation| {
                        let player = invocation.player().map(|p| p.id.as_u64());
                        let line = script::command_line(&name, &invocation.args, player);
                        script::observe(&log(), &line, &Action::Record);
                    })
                    .register();
            }
            Directive::Fire {
                command,
                event,
                payload,
            } => {
                let label = command.clone();
                let _ = ctx
                    .command(&label)
                    .handler(move |_| {
                        let line = match Context::new().fire_named_text(&event, &payload) {
                            Ok(outcome) => script::fired_line(
                                &command,
                                &event,
                                outcome.cancelled,
                                outcome.response.as_ref().and_then(NamedResponse::text),
                            ),
                            Err(error) => format!("cmd {command} failed {error}"),
                        };
                        script::append(&log(), &line);
                    })
                    .register();
            }
            Directive::Named {
                name,
                priority,
                action,
            } => subscribe_named(ctx, name, priority, action),
            Directive::Channel { id } => {
                Messaging::register(&ChannelId::modern(id))?;
            }
            Directive::Config { key } => {
                let line = match Config::get(&key) {
                    Ok(value) => script::config_line(&key, value.as_deref()),
                    Err(error) => format!("config {key} failed {error}"),
                };
                script::append(&log(), &line);
            }
        }
    }
    let line = match ctx.enable_reason() {
        Some(EnableReason::Recovered(info)) => format!("enable recovered {}", info.attempt),
        _ => "enable".to_owned(),
    };
    script::append(&log(), &line);
    Ok(())
}

pub fn disable() -> Result<(), PluginError> {
    script::append(&log(), "disable");
    Ok(())
}

fn subscribe(ctx: &Context, event: EventName, priority: u8, action: Action) {
    let cancel = action == Action::Cancelled;
    let at = EventPriority::Custom(priority);
    let subscription = match event {
        EventName::PreLogin => ctx.on::<PreLoginEvent>(at, move |e| {
            let uuid = e.profile.uuid.to_string();
            let remote = e.remote_addr.to_string();
            let protocol = e.protocol.to_string();
            let fields = [
                e.profile.username.as_str(),
                &uuid,
                &remote,
                &protocol,
                &e.server_domain,
            ];
            seen(event, priority, &fields, &action);
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::ForceOffline => e.force_offline(),
                Action::ForceOnline => e.force_online(),
                _ => {}
            }
        }),
        EventName::PostLogin => ctx.on::<PostLoginEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let uuid = e.profile.uuid.to_string();
            let protocol = e.protocol.to_string();
            let fields = [id.as_str(), &e.profile.username, &uuid, &protocol];
            seen(event, priority, &fields, &action);
        }),
        EventName::Disconnect => ctx.on::<DisconnectEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let last = or_dash(e.last_server.as_ref().map(ServerId::as_str));
            let fields = [
                id.as_str(),
                &e.player.username,
                last,
                disconnect_cause(&e.cause),
            ];
            seen(event, priority, &fields, &action);
        }),
        EventName::OnlineAuthFailed => ctx.on::<OnlineAuthFailedEvent>(at, move |e| {
            seen(event, priority, &[&e.username], &action);
        }),
        EventName::PermissionsSetup => ctx.on::<PermissionsSetupEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let online = e.online_mode.to_string();
            seen(
                event,
                priority,
                &[&id, &e.player.username, &online],
                &action,
            );
            if let Action::Custom(level) = &action {
                e.provide(if level == "admin" {
                    PermissionSnapshot::admin()
                } else {
                    PermissionSnapshot::new().deny("*")
                });
            }
        }),
        EventName::ServerPreConnect => ctx.on::<ServerPreConnectEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let previous = or_dash(e.previous_server.as_ref().map(ServerId::as_str));
            let fields = [
                id.as_str(),
                &e.player.username,
                e.server.as_str(),
                previous,
                connect_cause(e.cause),
            ];
            seen(event, priority, &fields, &action);
            match &action {
                Action::Allow => e.allow(),
                Action::ConnectTo(server) => e.redirect_to(server.as_str()),
                Action::Limbo(handlers) => e.send_to_limbo(handlers.clone()),
                Action::Deny(reason) => e.deny(text(reason)),
                _ => {}
            }
        }),
        EventName::ServerConnected => ctx.on::<ServerConnectedEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let previous = or_dash(e.previous_server.as_ref().map(ServerId::as_str));
            seen(
                event,
                priority,
                &[&id, e.server.as_str(), previous],
                &action,
            );
        }),
        EventName::ServerPostConnect => ctx.on::<ServerPostConnectEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let previous = or_dash(e.previous_server.as_ref().map(ServerId::as_str));
            seen(
                event,
                priority,
                &[&id, e.server.as_str(), previous],
                &action,
            );
        }),
        EventName::KickedFromServer => ctx.on::<KickedFromServerEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let reason = e.reason.as_ref().map_or_else(|| "-".to_owned(), json);
            let during = e.during_connect.to_string();
            let previous = or_dash(e.previous_server.as_ref().map(ServerId::as_str));
            let fields = [
                id.as_str(),
                e.server.as_str(),
                &reason,
                kick_cause(&e.cause),
                &during,
                previous,
            ];
            seen(event, priority, &fields, &action);
            match &action {
                Action::Redirect(server) => e.redirect_to(server.as_str()),
                Action::Limbo(handlers) => e.send_to_limbo(handlers.clone()),
                Action::Notify(message) => e.notify(text(message)),
                Action::Disconnect(reason) => e.disconnect(text(reason)),
                _ => {}
            }
        }),
        EventName::PlayerChooseInitialServer => {
            ctx.on::<PlayerChooseInitialServerEvent>(at, move |e| {
                let id = e.player.id.to_string();
                let fields = [id.as_str(), &e.player.username, e.initial_server.as_str()];
                seen(event, priority, &fields, &action);
                match &action {
                    Action::Allow => e.allow(),
                    Action::Redirect(server) => e.redirect_to(server.as_str()),
                    Action::Limbo(handlers) => e.send_to_limbo(handlers.clone()),
                    _ => {}
                }
            })
        }
        EventName::ProxyPing => ctx.on::<ProxyPingEvent>(at, move |e| {
            let remote = e.remote_addr.to_string();
            let protocol = e.protocol.to_string();
            let legacy = e.legacy.to_string();
            let response = e.response();
            let description = json(&response.description);
            let max = response.max_players.to_string();
            let online = response.online_players.to_string();
            let answered = response.protocol.to_string();
            let fields = [
                remote.as_str(),
                or_dash(e.server.as_ref().map(ServerId::as_str)),
                or_dash(e.virtual_host.as_deref()),
                &protocol,
                &legacy,
                &description,
                &max,
                &online,
                &answered,
                &response.version_name,
                or_dash(response.favicon.as_deref()),
            ];
            seen(event, priority, &fields, &action);
            if let Action::Description(description) = &action {
                e.response_mut().description = text(description);
            }
        }),
        EventName::ProxyInitialize => ctx.on::<ProxyInitializeEvent>(at, move |_| {
            seen(event, priority, &[], &action);
        }),
        EventName::ProxyShutdown => ctx.on::<ProxyShutdownEvent>(at, move |_| {
            seen(event, priority, &[], &action);
        }),
        EventName::ConfigReload => ctx.on::<ConfigReloadEvent>(at, move |e| {
            let fields = [
                e.provider.clone(),
                joined(e.added.iter()),
                joined(e.removed.iter()),
                joined(e.updated.iter()),
            ];
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen(event, priority, &fields, &action);
        }),
        EventName::ServerStateChange => ctx.on::<ServerStateChangeEvent>(at, move |e| {
            let fields = [e.server.as_str(), state(e.old_state), state(e.new_state)];
            seen(event, priority, &fields, &action);
        }),
        EventName::ChatMessage => ctx.on::<ChatMessageEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let signed = e.signed.to_string();
            let server = or_dash(e.server.as_ref().map(ServerId::as_str));
            seen(
                event,
                priority,
                &[&id, &e.message, &signed, server],
                &action,
            );
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::Modify(message) => e.modify(message.as_str()),
                _ => {}
            }
        }),
        EventName::BackendHealth => ctx.on::<BackendHealthEvent>(at, move |e| {
            let address = e.address.to_string();
            let servers = joined(e.servers.iter());
            seen(
                event,
                priority,
                &[&address, &servers, backend_state(e.state)],
                &action,
            );
        }),
        EventName::Login => ctx.on::<LoginEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let online = e.online_mode.to_string();
            seen(
                event,
                priority,
                &[&id, &e.player.username, &online],
                &action,
            );
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                _ => {}
            }
        }),
        EventName::GameProfileRequest => ctx.on::<GameProfileRequestEvent>(at, move |e| {
            let uuid = e.original.uuid.to_string();
            let online = e.online_mode.to_string();
            let remote = e.remote_addr.to_string();
            let protocol = e.protocol.to_string();
            let fields = [
                e.original.username.as_str(),
                &uuid,
                &online,
                &remote,
                or_dash(e.virtual_host.as_deref()),
                &protocol,
                e.profile().username.as_str(),
            ];
            seen(event, priority, &fields, &action);
            if let Action::Rename(name) = &action {
                e.profile_mut().username = name.clone();
            }
        }),
        EventName::CommandExecute => ctx.on::<CommandExecuteEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let signed = e.signed.to_string();
            let server = or_dash(e.server.as_ref().map(ServerId::as_str));
            seen(
                event,
                priority,
                &[&id, &e.command, &signed, server],
                &action,
            );
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::Modify(command) => e.modify(command.as_str()),
                Action::ForwardToBackend => e.forward_to_backend(),
                _ => {}
            }
        }),
        EventName::ConnectionHandshake => ctx.on::<ConnectionHandshakeEvent>(at, move |e| {
            let remote = e.remote_addr.to_string();
            let port = e.port.to_string();
            let protocol = e.protocol.to_string();
            let intent = match e.intent {
                HandshakeIntent::Status => "status",
                HandshakeIntent::Transfer => "transfer",
                _ => "login",
            };
            let legacy = e.legacy.to_string();
            let fields = [
                remote.as_str(),
                or_dash(e.virtual_host.as_deref()),
                &e.raw_host,
                &port,
                &protocol,
                intent,
                &legacy,
                or_dash(e.server.as_ref().map(ServerId::as_str)),
            ];
            seen(event, priority, &fields, &action);
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::Drop => e.drop_silently(),
                _ => {}
            }
        }),
        EventName::ConnectionRejected => ctx.on::<ConnectionRejectedEvent>(at, move |e| {
            let remote = e.remote_addr.to_string();
            let reason = reject_reason(&e.reason);
            let fields = [remote.as_str(), or_dash(e.virtual_host.as_deref()), &reason];
            seen(event, priority, &fields, &action);
        }),
        EventName::LimboEnter => ctx.on::<LimboEnterEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let handlers = joined(e.handlers.iter());
            let context = limbo_context(&e.context);
            seen(event, priority, &[&id, &handlers, &context], &action);
        }),
        EventName::LimboExit => ctx.on::<LimboExitEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let next = or_dash(e.next_server.as_ref().map(ServerId::as_str));
            seen(
                event,
                priority,
                &[&id, limbo_exit(&e.reason), next],
                &action,
            );
        }),
        EventName::PlayerClientBrand => ctx.on::<PlayerClientBrandEvent>(at, move |e| {
            let id = e.player.id.to_string();
            seen(event, priority, &[&id, &e.brand], &action);
        }),
        EventName::PlayerSettingsChanged => ctx.on::<PlayerSettingsChangedEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let view = e.settings.view_distance.to_string();
            let hand = match e.settings.main_hand {
                MainHand::Left => "left",
                _ => "right",
            };
            seen(
                event,
                priority,
                &[&id, &e.settings.locale, &view, hand],
                &action,
            );
        }),
        EventName::PlayerChannelRegister => ctx.on::<PlayerChannelRegisterEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let channels = joined(e.channels.iter());
            seen(
                event,
                priority,
                &[&id, &channels, direction(e.direction)],
                &action,
            );
        }),
        EventName::PluginMessage => ctx.on::<PluginMessageEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let source = match &e.source {
                MessageEndpoint::Backend(server) => format!("backend:{server}"),
                _ => "client".to_owned(),
            };
            let [modern, legacy] = channel_fields(&e.channel);
            let data = bytes_text(&e.data);
            let phase = match e.phase {
                MessagePhase::Configuration => "configuration",
                _ => "play",
            };
            let fields = [
                id.as_str(),
                &source,
                &modern,
                &legacy,
                &e.raw_channel,
                &data,
                phase,
            ];
            seen(event, priority, &fields, &action);
            match &action {
                Action::Forward => e.forward(),
                Action::Handled => e.handled(),
                Action::Replace(data) => e.replace(data.as_bytes()),
                Action::Reply(data) => {
                    let _ = Messaging::send_to_player(e.player.id, &e.channel, data.as_bytes());
                    e.handled();
                }
                _ => {}
            }
        }),
        EventName::BanIssued => ctx.on::<BanIssuedEvent>(at, move |e| {
            let fields = ban_fields(&e.entry, &e.source, e.silent);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen(event, priority, &fields, &action);
        }),
        EventName::BanRevoked => ctx.on::<BanRevokedEvent>(at, move |e| {
            let fields = ban_fields(&e.entry, &e.source, e.silent);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen(event, priority, &fields, &action);
        }),
        EventName::PluginEnabled => ctx.on::<PluginEnabledEvent>(at, move |e| {
            seen(event, priority, &[&e.plugin_id, &e.version], &action);
        }),
        EventName::PluginDisabled => ctx.on::<PluginDisabledEvent>(at, move |e| {
            seen(event, priority, &[&e.plugin_id], &action);
        }),
        EventName::PreTransfer => ctx.on::<PreTransferEvent>(at, move |e| {
            let id = e.player.id.to_string();
            let port = e.port.to_string();
            let origin = match e.origin {
                TransferOrigin::Backend => "backend",
                _ => "plugin",
            };
            seen(event, priority, &[&id, &e.host, &port, origin], &action);
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::Redirect(target) => {
                    if let Some((host, port)) = target.rsplit_once(':')
                        && let Ok(port) = port.parse()
                    {
                        e.redirect(host, port);
                    }
                }
                _ => {}
            }
        }),
        EventName::PlayerResourcePackStatus => {
            ctx.on::<PlayerResourcePackStatusEvent>(at, move |e| {
                let id = e.player.id.to_string();
                let pack = e
                    .pack_id
                    .map_or_else(|| "-".to_owned(), |pack| pack.to_string());
                let origin = match e.origin {
                    ResourcePackOrigin::Backend => "backend",
                    _ => "proxy",
                };
                seen(
                    event,
                    priority,
                    &[&id, &pack, pack_status(e.status), origin],
                    &action,
                );
            })
        }
        EventName::NamedEvent => ctx.on::<NamedEvent>(at, move |e| {
            let mut fields = vec![e.name.clone()];
            fields.extend(named_fields(e));
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            seen(event, priority, &fields, &action);
            answer_named(e, &action);
        }),
        EventName::RawPacket => ctx.on_packets(
            &[PacketFilter::serverbound(
                script::PACKET_ID,
                ConnectionState::Play,
            )],
            at,
            move |e| {
                let id = e.player.to_string();
                let packet = e.packet_id.to_string();
                let data = bytes_text(&e.data);
                seen(
                    event,
                    priority,
                    &[&id, direction(e.direction), &packet, &data],
                    &action,
                );
                match &action {
                    Action::Pass => e.pass(),
                    Action::Drop => e.drop_packet(),
                    Action::Modify(data) => e.modify(e.packet_id, data.as_bytes()),
                    _ => {}
                }
            },
        ),
    };
    if cancel && let Ok(subscription) = subscription {
        subscription.cancel();
    }
}
