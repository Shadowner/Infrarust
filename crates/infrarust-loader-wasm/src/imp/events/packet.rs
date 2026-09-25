use infrarust_api::events::packet::{RawPacketEvent, RawPacketResult};

use super::{Applied, WasmEvent, unmatched};
use crate::bindings::infrarust::plugin::events::{self as we, EventKind};
use crate::convert;

impl WasmEvent for RawPacketEvent {
    const KIND: EventKind = EventKind::RawPacket;

    fn to_wit(&self) -> we::Event {
        we::Event::RawPacket(we::RawPacketEvent {
            player: self.player_id.as_u64(),
            direction: convert::packet_direction_to_wit(self.direction),
            packet: convert::raw_packet_to_wit(&self.packet),
            result: match self.result() {
                RawPacketResult::Modify { packet } => {
                    we::RawPacketResult::Modify(convert::raw_packet_to_wit(packet))
                }
                RawPacketResult::Drop => we::RawPacketResult::Drop,
                _ => we::RawPacketResult::Pass,
            },
        })
    }

    fn apply(&mut self, outcome: we::EventOutcome) -> Applied {
        let we::EventOutcome::RawPacket(result) = outcome else {
            return unmatched(&outcome);
        };
        self.set_result(match result {
            we::RawPacketResult::Pass => RawPacketResult::Pass,
            we::RawPacketResult::Modify(packet) => RawPacketResult::Modify {
                packet: convert::raw_packet_from_wit(packet),
            },
            we::RawPacketResult::Drop => RawPacketResult::Drop,
        });
        Applied::Set
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use infrarust_api::events::packet::PacketDirection;
    use infrarust_api::types::{PlayerId, RawPacket};

    use super::*;
    use crate::bindings::infrarust::plugin::types as wt;

    #[test]
    fn a_packet_outcome_modifies_drops_or_passes_it() {
        let mut event = RawPacketEvent::new(
            PlayerId::new(3),
            PacketDirection::Serverbound,
            RawPacket::new(0x05, Bytes::from_static(b"abc")),
        );
        let we::Event::RawPacket(record) = event.to_wit() else {
            panic!("a packet is sent as raw-packet");
        };
        assert_eq!(record.player, 3);
        assert_eq!(record.packet.data, b"abc");
        assert_eq!(record.result, we::RawPacketResult::Pass);

        event.apply(we::EventOutcome::RawPacket(we::RawPacketResult::Modify(
            wt::RawPacket {
                packet_id: 0x06,
                data: b"xyz".to_vec(),
            },
        )));
        assert!(matches!(
            event.result(),
            RawPacketResult::Modify { packet } if packet.packet_id == 0x06 && packet.data == Bytes::from_static(b"xyz")
        ));
        event.apply(we::EventOutcome::RawPacket(we::RawPacketResult::Drop));
        assert!(matches!(event.result(), RawPacketResult::Drop));
        event.apply(we::EventOutcome::RawPacket(we::RawPacketResult::Pass));
        assert!(matches!(event.result(), RawPacketResult::Pass));
    }
}
