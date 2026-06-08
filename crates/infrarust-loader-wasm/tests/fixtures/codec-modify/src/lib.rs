use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecModify;

struct OpFilter;

impl CodecFilter for OpFilter {
    fn filter(&mut self, _ctx: &CodecContext, packet: &mut Packet, out: &mut Injections) -> Verdict {
        match packet.id() {
            0x01 => Verdict::Drop,
            0x02 => {
                packet.set_data(b"MODIFIED".to_vec());
                Verdict::Pass
            }
            0x03 => {
                out.before(Packet::new(0xfe, b"before".to_vec()));
                out.after(Packet::new(0xff, b"after".to_vec()));
                Verdict::Pass
            }
            _ => Verdict::Pass,
        }
    }
}

#[plugin(id = "codec-modify", name = "Codec Modify Fixture")]
impl Plugin for CodecModify {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("ops", FilterPriority::Normal, |_init| Box::new(OpFilter));
    }
}
