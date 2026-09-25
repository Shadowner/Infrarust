use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

const UNAVAILABLE: &str = "ban check unavailable";

fn log(line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/log.txt")
        .expect("opening the fixture log");
    writeln!(file, "{line}").expect("writing the fixture log");
}

fn ban_check(username: &str) -> Result<bool, ServiceError> {
    Bans.is_banned(&BanTarget::Username(username.to_string()))
}

#[derive(Default)]
struct DeadlineProbe;

struct BanGate;

impl LimboHandler for BanGate {
    fn on_player_enter(&self, session: &LimboSession) -> HandlerOutcome {
        match ban_check(&session.profile().username) {
            Ok(false) => HandlerOutcome::Accept,
            Ok(true) => HandlerOutcome::Deny(Component::text("Banned")),
            Err(_) => HandlerOutcome::Deny(Component::text(UNAVAILABLE)),
        }
    }
}

#[plugin(id = "deadline-probe", name = "Deadline Probe Fixture")]
impl Plugin for DeadlineProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PreLoginEvent>(EventPriority::Normal, |event| {
            match ban_check(&event.profile.username) {
                Ok(false) => log("pre-login allowed"),
                Ok(true) => event.deny(Component::text("Banned").into_json()),
                Err(_) => {
                    log("pre-login service-error");
                    event.deny(Component::text(UNAVAILABLE).into_json());
                }
            }
        });
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            log("pre-connect");
            event.redirect_to("backend-1");
        });
        ctx.command("check", |_| {
            log(match ban_check("Steve") {
                Ok(_) => "check answered",
                Err(_) => "check service-error",
            });
        })
        .register();
        ctx.command("ping", |_| log("command")).register();
        Ok(())
    }

    fn register_limbo_handlers(reg: &mut LimboRegistrar) {
        reg.add("ban-gate", BanGate);
    }
}
