use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use infrarust_plugin_sdk::prelude::*;

const BALLAST_BYTES: usize = 64 * 1024;
const ATTACKER: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

enum Fate {
    Healthy,
    TrapAt(u64),
    SpinAt(u64),
}

impl Fate {
    fn for_connection(faulty: bool, init: &CodecSessionInit) -> Self {
        if !faulty || !is_attacker(init.remote_addr) {
            return Self::Healthy;
        }
        match init.side {
            ConnectionSide::ClientSide => Self::TrapAt(3 + init.connection_id % 20),
            ConnectionSide::ServerSide => Self::SpinAt(2),
        }
    }
}

struct PassThrough {
    packets: u64,
    fate: Fate,
    ballast: Vec<u8>,
}

impl CodecFilter for PassThrough {
    fn filter(
        &mut self,
        _ctx: &CodecContext,
        _packet: &mut Packet,
        _out: &mut Injections,
    ) -> Verdict {
        self.packets += 1;
        let slot = usize::try_from(self.packets).unwrap_or(0) % self.ballast.len();
        self.ballast[slot] = self.ballast[slot].wrapping_add(1);
        match self.fate {
            Fate::TrapAt(n) if self.packets == n => {
                panic!("soak-codec: trap at packet {n}");
            }
            Fate::SpinAt(n) if self.packets == n => loop {
                std::hint::black_box(0u64);
            },
            _ => Verdict::Pass,
        }
    }
}

fn is_attacker(addr: SocketAddr) -> bool {
    addr.ip() == ATTACKER
}

pub fn register(reg: &mut CodecRegistrar, id: &str, faulty: bool) {
    reg.add(id, FilterPriority::Normal, move |init| {
        Box::new(PassThrough {
            packets: 0,
            fate: Fate::for_connection(faulty, init),
            ballast: vec![1u8; BALLAST_BYTES],
        })
    });
}
