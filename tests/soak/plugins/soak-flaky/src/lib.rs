mod behaviour;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SoakFlaky;

#[plugin(id = "soak-flaky", name = "Soak Flaky")]
impl Plugin for SoakFlaky {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        behaviour::install(ctx, "soak-flaky", "fcmd")
    }
}
