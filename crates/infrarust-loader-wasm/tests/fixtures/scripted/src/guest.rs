use std::path::{Path, PathBuf};

use infrarust_plugin_sdk::prelude::*;

use crate::script::{self, Action, Directive, EventName, joined, or_dash};

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
    };
    if cancel && let Ok(subscription) = subscription {
        subscription.cancel();
    }
}
