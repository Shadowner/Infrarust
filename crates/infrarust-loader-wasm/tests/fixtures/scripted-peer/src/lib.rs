#[path = "../../scripted/src/guest.rs"]
mod guest;
#[path = "../../scripted/src/script.rs"]
mod script;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct ScriptedPeer;

#[plugin(id = "scripted-peer", name = "Scripted Peer Fixture")]
impl Plugin for ScriptedPeer {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        guest::enable(ctx)
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        guest::disable()
    }
}
