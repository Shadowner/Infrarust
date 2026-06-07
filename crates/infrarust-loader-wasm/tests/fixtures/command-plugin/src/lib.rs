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
        .register();
        Ok(())
    }
}
