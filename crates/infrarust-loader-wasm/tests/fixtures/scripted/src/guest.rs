use std::path::{Path, PathBuf};

use infrarust_plugin_sdk::prelude::*;

use crate::script::{self, Action, Directive, EventName};

fn log() -> PathBuf {
    Path::new("/").join(script::LOG_FILE)
}

fn text(message: &str) -> String {
    Component::text(message).into_json()
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
    }
}

pub fn enable(ctx: &Context) -> Result<(), String> {
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
                let key = name.clone();
                ctx.command(&key, move |invocation| {
                    let line = script::command_line(&name, &invocation.args, invocation.player);
                    script::observe(&log(), &line, &Action::Record);
                })
                .register();
            }
        }
    }
    script::append(&log(), "enable");
    Ok(())
}

pub fn disable() -> Result<(), String> {
    script::append(&log(), "disable");
    Ok(())
}

fn subscribe(ctx: &Context, event: EventName, priority: u8, action: Action) {
    let cancel = action == Action::Cancelled;
    let at = EventPriority::Custom(priority);
    let subscription = match event {
        EventName::PreLogin => ctx.on::<PreLoginEvent>(at, move |e| {
            let protocol = e.protocol_version.to_string();
            let fields = [
                e.profile.username.as_str(),
                &e.profile.uuid,
                &e.remote_addr,
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
            let id = e.player_id.to_string();
            let protocol = e.protocol_version.to_string();
            let fields = [id.as_str(), &e.profile.username, &e.profile.uuid, &protocol];
            seen(event, priority, &fields, &action);
        }),
        EventName::Disconnect => ctx.on::<DisconnectEvent>(at, move |e| {
            let id = e.player_id.to_string();
            let last = e.last_server.as_deref().unwrap_or("-");
            seen(event, priority, &[&id, &e.username, last], &action);
        }),
        EventName::OnlineAuthFailed => ctx.on::<OnlineAuthFailedEvent>(at, move |e| {
            seen(event, priority, &[&e.username], &action);
        }),
        EventName::PermissionsSetup => ctx.on::<PermissionsSetupEvent>(at, move |e| {
            let id = e.player_id.to_string();
            let online = e.online_mode.to_string();
            seen(
                event,
                priority,
                &[&id, &e.profile.username, &online],
                &action,
            );
        }),
        EventName::ServerPreConnect => ctx.on::<ServerPreConnectEvent>(at, move |e| {
            let id = e.player_id.to_string();
            let fields = [id.as_str(), &e.profile.username, &e.original_server];
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
            let id = e.player_id.to_string();
            seen(event, priority, &[&id, &e.server], &action);
        }),
        EventName::ServerSwitch => ctx.on::<ServerSwitchEvent>(at, move |e| {
            let id = e.player_id.to_string();
            let fields = [id.as_str(), &e.previous_server, &e.new_server];
            seen(event, priority, &fields, &action);
        }),
        EventName::KickedFromServer => ctx.on::<KickedFromServerEvent>(at, move |e| {
            let id = e.player_id.to_string();
            seen(event, priority, &[&id, &e.server, &e.reason], &action);
            match &action {
                Action::Redirect(server) => e.redirect_to(server.as_str()),
                Action::Limbo(handlers) => e.send_to_limbo(handlers.clone()),
                Action::Notify(message) => e.notify(text(message)),
                Action::Disconnect(reason) => e.disconnect_player(text(reason)),
                _ => {}
            }
        }),
        EventName::PlayerChooseInitialServer => {
            ctx.on::<PlayerChooseInitialServerEvent>(at, move |e| {
                let id = e.player_id.to_string();
                let fields = [id.as_str(), &e.profile.username, &e.initial_server];
                seen(event, priority, &fields, &action);
                match &action {
                    Action::Allow => e.allow(),
                    Action::Redirect(server) => e.redirect(server.as_str()),
                    Action::Limbo(handlers) => e.send_to_limbo(handlers.clone()),
                    _ => {}
                }
            })
        }
        EventName::ProxyPing => ctx.on::<ProxyPingEvent>(at, move |e| {
            let response = &e.response;
            let max = response.max_players.to_string();
            let online = response.online_players.to_string();
            let protocol = response.protocol_version.to_string();
            let fields = [
                e.remote_addr.as_str(),
                &response.description,
                &max,
                &online,
                &protocol,
                &response.version_name,
                response.favicon.as_deref().unwrap_or("-"),
            ];
            seen(event, priority, &fields, &action);
            if let Action::Description(description) = &action {
                e.response.description = text(description);
            }
        }),
        EventName::ProxyInitialize => ctx.on::<ProxyInitializeEvent>(at, move |_| {
            seen(event, priority, &[], &action);
        }),
        EventName::ProxyShutdown => ctx.on::<ProxyShutdownEvent>(at, move |_| {
            seen(event, priority, &[], &action);
        }),
        EventName::ConfigReload => ctx.on::<ConfigReloadEvent>(at, move |_| {
            seen(event, priority, &[], &action);
        }),
        EventName::ServerStateChange => ctx.on::<ServerStateChangeEvent>(at, move |e| {
            let fields = [e.server.as_str(), state(e.old_state), state(e.new_state)];
            seen(event, priority, &fields, &action);
        }),
        EventName::ChatMessage => ctx.on::<ChatMessageEvent>(at, move |e| {
            let id = e.player_id.to_string();
            seen(event, priority, &[&id, &e.message], &action);
            match &action {
                Action::Allow => e.allow(),
                Action::Deny(reason) => e.deny(text(reason)),
                Action::Modify(message) => e.modify(message.as_str()),
                _ => {}
            }
        }),
    };
    if cancel {
        subscription.cancel();
    }
}
