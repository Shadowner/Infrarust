//! {{project-name}} an Infrarust WASM plugin.

#![forbid(unsafe_code)]

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct {{crate_name | pascal_case}};

#[plugin]
impl Plugin for {{crate_name | pascal_case}} {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!("{} joined", event.player.username);
        })?;

        ctx.command("hello")
            .description("Say hello")
            .handler(|invocation| {
                let who = invocation.args.first().map_or("world", String::as_str);
                let _ = invocation.reply(Component::text(format!("hello, {who}!")));
            })
            .register()?;

        Ok(())
    }
}
