use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandPlugin;

#[plugin(id = "command-plugin", name = "Command Plugin Fixture")]
impl Plugin for CommandPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("greet")
            .description("Greets the caller")
            .handler(|invocation| {
                let _ = std::fs::write("command.marker", invocation.args.join(","));
            })
            .completer(|completion| {
                let partial = completion.partial();
                ["world", "everyone", "friend"]
                    .into_iter()
                    .filter(|candidate| candidate.starts_with(partial))
                    .collect::<Vec<_>>()
            })
            .register()?;
        ctx.command("nest")
            .description("Registers `nested` from inside its own completer")
            .completer(|_| {
                let _ = Context::new()
                    .command("nested")
                    .handler(|_| {
                        let _ = std::fs::write("nested.marker", "ran");
                    })
                    .completer(|_| vec!["inner"])
                    .register();
                vec!["registered"]
            })
            .register()?;
        ctx.command("unnest")
            .description("Unregisters `nested` through the SDK")
            .handler(|_| {
                let removed = Context::new().unregister_command("nested").unwrap_or(false);
                let _ = std::fs::write("unnest.marker", removed.to_string());
            })
            .register()?;
        Ok(())
    }
}
