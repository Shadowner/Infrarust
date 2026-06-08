//! Codec fixture with per-connection state: counts the packets it sees and writes
//! the running count into each payload. The host test drives the client-side and
//! server-side instances separately to prove their state is independent.

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecStateful;

struct Counter {
    count: u32,
}

impl CodecFilter for Counter {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        self.count += 1;
        packet.set_data(self.count.to_le_bytes().to_vec());
        Verdict::Pass
    }
}

#[plugin(id = "codec-stateful", name = "Codec Stateful Fixture")]
impl Plugin for CodecStateful {
    fn on_enable(&self, _ctx: &Context) -> Result<(), String> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("counter", FilterPriority::Normal, |_init| {
            Box::new(Counter { count: 0 })
        });
    }
}
