use std::fs::OpenOptions;
use std::io::Write;

use infrarust_plugin_sdk::prelude::*;

const LOG: &str = "peer.log";
const CHANNEL: &str = "sec:peer";

#[derive(Default)]
struct SecPeer;

struct PassThrough;

impl CodecFilter for PassThrough {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        _packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        Verdict::Pass
    }
}

#[plugin(id = "sec-peer", name = "Security Peer Fixture")]
impl Plugin for SecPeer {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        let _ = Messaging::register(&ChannelId::modern(CHANNEL));
        ctx.command("peer-cmd")
            .description("A command owned by sec-peer")
            .handler(|invocation| {
                let outcome = match invocation.args.first().map(String::as_str) {
                    Some("channels") => Messaging::channels()
                        .map(|list| list.len().to_string())
                        .unwrap_or_else(|error| format!("err {error:?}")),
                    _ => "peer-cmd-ran".to_owned(),
                };
                if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(LOG) {
                    let _ = log.write_all(format!("{outcome}\n").as_bytes());
                }
            })
            .register()?;
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("peer-filter", FilterPriority::Normal, |_| {
            Box::new(PassThrough)
        });
    }
}
