//! {{project-name}} an Infrarust WASM plugin.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct {{crate_name | pascal_case}};

#[plugin]
impl Plugin for {{crate_name | pascal_case}} {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        // React to players joining.
        ctx.on::<PostLoginEvent>(EventPriority::Normal, |event| {
            info!("{} joined", event.profile.username);
        });

        // Register a "/hello [name]" command.
        ctx.command("hello", |invocation| {
            let who = invocation.args.first().map_or("world", String::as_str);
            info!("hello, {who}!");
        })
        .description("Say hello")
        .register();

        Ok(())
    }
}
