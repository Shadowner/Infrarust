mod guest;
mod script;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct Scripted;

#[plugin(id = "scripted", name = "Scripted Fixture")]
impl Plugin for Scripted {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        guest::enable(ctx)
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        guest::disable()
    }
}
