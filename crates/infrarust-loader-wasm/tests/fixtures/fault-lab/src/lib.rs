mod faults;
mod lab;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct FaultLab;

#[plugin(id = "fault-lab", name = "Fault Lab Fixture")]
impl Plugin for FaultLab {
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
