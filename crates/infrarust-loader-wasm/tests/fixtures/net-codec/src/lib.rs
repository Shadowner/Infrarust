use std::net::TcpStream;

use infrarust_plugin_sdk::prelude::*;
use wasip2::http::outgoing_handler;
use wasip2::http::types::{Fields, OutgoingRequest, Scheme};

#[derive(Default)]
struct NetCodec;

struct Reach;

impl CodecFilter for Reach {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        let request = String::from_utf8_lossy(packet.data()).into_owned();
        let outcome = match request.split_once(' ') {
            Some(("echo", text)) => text.to_owned(),
            Some(("tcp", target)) => match TcpStream::connect(target) {
                Ok(_) => "connected".to_owned(),
                Err(error) => format!("{:?}", error.kind()),
            },
            Some(("http", authority)) => {
                let outgoing = OutgoingRequest::new(Fields::new());
                let _ = outgoing.set_scheme(Some(&Scheme::Http));
                let _ = outgoing.set_authority(Some(authority));
                match outgoing_handler::handle(outgoing, None) {
                    Ok(_) => "sent".to_owned(),
                    Err(code) => format!("{code:?}"),
                }
            }
            _ => "unknown".to_owned(),
        };
        packet.set_data(outcome.into_bytes());
        Verdict::Pass
    }
}

#[plugin(id = "net-codec", name = "Network Codec Fixture")]
impl Plugin for NetCodec {
    fn on_enable(&self, _ctx: &Context) -> Result<(), PluginError> {
        Ok(())
    }

    fn register_codec_filters(reg: &mut CodecRegistrar) {
        reg.add("reach", FilterPriority::Normal, |_| Box::new(Reach));
    }
}
