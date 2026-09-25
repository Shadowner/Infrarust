use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

fn log(line: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/log.txt")
        .expect("opening the fixture log");
    writeln!(file, "{line}").expect("writing the fixture log");
}

#[derive(Default)]
struct SlowHandler;

#[plugin(id = "slow-handler", name = "Slow Handler Fixture")]
impl Plugin for SlowHandler {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            let answer = Bans.is_banned(&BanTarget::Username(event.profile.username.clone()));
            log(match answer {
                Ok(_) => "post-login answered",
                Err(_) => "post-login service-error",
            });
        });
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            log("pre-connect");
            event.redirect_to("backend-1");
        });
        ctx.command("ping", |_| log("command"))
            .completer(|_partial, _cursor| vec!["pong".to_string()])
            .register();
        Ok(())
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), String> {
        log("disable");
        Ok(())
    }
}
