use std::collections::HashMap;
use std::time::Instant;

use infrarust_plugin_sdk::prelude::*;

#[derive(Default)]
struct CodecStd;

struct Tally {
    started: Instant,
    seen: HashMap<i32, u32>,
}

impl CodecFilter for Tally {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        let count = self.seen.entry(packet.id()).or_insert(0);
        *count += 1;
        let count = *count;
        info!(
            "codec-std saw packet {:#x} {count} times after {:?}",
            packet.id(),
            self.started.elapsed()
        );
        packet.set_data(count.to_le_bytes().to_vec());
        Verdict::Pass
    }
}

#[plugin(id = "codec-std", name = "Codec Std Fixture")]
impl Plugin for CodecStd {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("codec-std-trap")
            .handler(|_| panic!("codec-std: trap on purpose"))
            .register()?;
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("tally", FilterPriority::Normal, |init| {
            info!(
                "codec-std filter created for connection {}",
                init.connection_id
            );
            println!("codec-std stdout for connection {}", init.connection_id);
            eprintln!("codec-std stderr for connection {}", init.connection_id);
            Box::new(Tally {
                started: Instant::now(),
                seen: HashMap::new(),
            })
        });
    }
}
