use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SlowHandler;

#[plugin(id = "slow-handler", name = "Slow Handler Fixture")]
impl Plugin for SlowHandler {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            let _ = Bans.is_banned(&BanTarget::Username(event.profile.username.clone()));
            let _ = std::fs::write("post-login.marker", b"ran");
        });
        ctx.on::<ServerPreConnectEvent>(EventPriority::Normal, |event| {
            let _ = std::fs::write("pre-connect.marker", b"ran");
            event.redirect_to("backend-1");
        });
        ctx.command("ping", |_| {
            let _ = std::fs::write("command.marker", b"ran");
        })
        .completer(|_partial, _cursor| vec!["pong".to_string()])
        .register();
        Ok(())
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), String> {
        std::fs::write("disable.marker", b"ran").map_err(|e| e.to_string())
    }
}
