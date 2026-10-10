#[path = "../../soak-flaky/src/behaviour.rs"]
mod behaviour;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SoakQuarantine;

#[plugin(id = "soak-quarantine", name = "Soak Quarantine")]
impl Plugin for SoakQuarantine {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        behaviour::install(ctx, "soak-quarantine", "qcmd")
    }
}
