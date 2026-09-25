//! WASM-2 `command-plugin` fixture, migrated to the SDK. Registers `greet` and
//! writes its args to the data_dir on invocation so the host test can confirm
//! the command dispatch reaches the guest.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandPlugin;

#[plugin(id = "command-plugin", name = "Command Plugin Fixture")]
impl Plugin for CommandPlugin {
    fn on_enable(&self, ctx: &Context) -> Result<(), String> {
        ctx.command("greet", |invocation| {
            let _ = std::fs::write("command.marker", invocation.args.join(",").as_bytes());
        })
        .description("Greets the caller")
        .completer(|partial, _cursor| {
            let p = partial.last().map(String::as_str).unwrap_or("");
            ["world", "everyone", "friend"]
                .iter()
                .filter(|c| c.starts_with(p))
                .map(|c| (*c).to_string())
                .collect()
        })
        .register();
        ctx.command("nest", |_| {})
            .description("Registers `nested` from inside its own completer")
            .completer(|_partial, _cursor| {
                Context::new()
                    .command("nested", |_| {
                        let _ = std::fs::write("nested.marker", b"ran");
                    })
                    .completer(|_partial, _cursor| vec!["inner".to_string()])
                    .register();
                vec!["registered".to_string()]
            })
            .register();
        ctx.command("unnest", |_| {
            let removed = Context::new().unregister_command("nested");
            let _ = std::fs::write("unnest.marker", removed.to_string().as_bytes());
        })
        .description("Unregisters `nested` through the SDK")
        .register();
        Ok(())
    }
}
