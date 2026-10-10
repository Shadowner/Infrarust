mod filter;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct SoakCodec;

#[plugin(id = "soak-codec", name = "Soak Codec")]
impl Plugin for SoakCodec {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        filter::register(reg, "soak-pass", false);
    }
}
