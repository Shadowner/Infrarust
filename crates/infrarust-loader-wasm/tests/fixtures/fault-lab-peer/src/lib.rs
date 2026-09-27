#[path = "../../fault-lab/src/faults.rs"]
mod faults;
#[path = "../../fault-lab/src/lab.rs"]
mod lab;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct FaultLabPeer;

#[plugin(id = "fault-lab-peer", name = "Fault Lab Peer Fixture")]
impl Plugin for FaultLabPeer {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        lab::enable(ctx)
    }

    fn on_disable(&self, _ctx: &Context) -> Result<(), PluginError> {
        lab::disable()
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        lab::codec_filters(reg);
    }

    fn register_limbo_handlers(reg: &mut LimboRegistrar) {
        lab::limbo_handlers(reg);
    }
}
