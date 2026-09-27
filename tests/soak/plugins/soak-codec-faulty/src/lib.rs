#[path = "../../soak-codec/src/filter.rs"]
mod filter;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SoakCodecFaulty;

#[plugin(id = "soak-codec-faulty", name = "Soak Codec Faulty")]
impl Plugin for SoakCodecFaulty {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        filter::register(reg, "soak-faulty", true);
    }
}
